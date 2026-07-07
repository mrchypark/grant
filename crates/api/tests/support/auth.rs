#![allow(dead_code)]

pub fn bearer_header(token: &str) -> String {
    format!("Bearer {token}")
}

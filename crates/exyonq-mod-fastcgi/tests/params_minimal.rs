/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
use exyonq_mod_fastcgi::{MinForwardRequest, ParamsError};

#[test]
fn get_builds_required_params() {
    let req = MinForwardRequest::get("/index.php?q=1", "/index.php", "/var/www/index.php");
    let params = req.to_fcgi_params().expect("params");
    assert!(params.iter().any(|(k, _)| k == "REQUEST_METHOD"));
    assert!(params.iter().any(|(k, _)| k == "REQUEST_URI"));
    assert!(params.iter().any(|(k, _)| k == "REMOTE_ADDR"));
    assert!(!params.iter().any(|(k, _)| k == "X-Forwarded-For"));
}

#[test]
fn post_includes_content_length_and_type() {
    let req = MinForwardRequest::post("/", "/", "/", "text/plain", b"body");
    let params = req.to_fcgi_params().expect("params");
    assert!(params
        .iter()
        .any(|(k, v)| k == "CONTENT_LENGTH" && v == "4"));
    assert!(params.iter().any(|(k, _)| k == "CONTENT_TYPE"));
}

#[test]
fn empty_request_method_rejected() {
    let mut req = MinForwardRequest::get("/", "/", "/");
    req.request_method = String::new();
    assert_eq!(
        req.to_fcgi_params().unwrap_err(),
        ParamsError::EmptyRequestMethod
    );
}

#[test]
fn query_string_included_when_present() {
    let mut req = MinForwardRequest::get("/?x=1", "/", "/");
    req.query_string = Some("x=1".to_string());
    let params = req.to_fcgi_params().expect("params");
    assert!(params
        .iter()
        .any(|(k, v)| k == "QUERY_STRING" && v == "x=1"));
}

#[test]
fn http_host_maps_via_headers() {
    let mut req = MinForwardRequest::get("/", "/", "/");
    req.http_headers
        .push(("Host".to_string(), "example.test".to_string()));
    let params = req.to_fcgi_params().expect("params");
    assert!(params.iter().any(|(k, _)| k == "HTTP_HOST"));
}

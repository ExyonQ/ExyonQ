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
//! Mechanical TLS reload port after certificate publication.

use crate::config::AppConfig;
use crate::reload;
use crate::tls::{SharedTlsAcceptor, TlsSessionCache};
use exyonq_module_api::acme_integration::CertificatePublicationPort;

pub struct CoreCertificatePublicationPort {
    pub tls_acceptor: SharedTlsAcceptor,
    pub tls_session_cache: TlsSessionCache,
}

impl CertificatePublicationPort for CoreCertificatePublicationPort {
    fn reload_tls_from_config(&self, config: &AppConfig) -> Result<(), String> {
        reload::reload_tls_acceptor(config, &self.tls_acceptor, &self.tls_session_cache)
            .map_err(|err| err.to_string())
    }
}

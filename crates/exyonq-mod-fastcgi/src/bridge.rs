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
//! Composition-root bridge — optional `core-bridge` feature only (no `exyonq-core` dep).

#[cfg(feature = "core-bridge")]
use exyonq_module_api::fcgi_dispatch::{
    FcgiDispatchService, FcgiRegisterError, FcgiRuntimeRegistration,
};
#[cfg(feature = "core-bridge")]
use exyonq_module_api::fcgi_script_resolver::{
    FastcgiScriptResolutionService, FastcgiScriptResolverRegisterError,
};
#[cfg(feature = "core-bridge")]
use exyonq_module_api::register_fastcgi_script_resolver;
#[cfg(feature = "core-bridge")]
use std::sync::Arc;

#[cfg(feature = "core-bridge")]
use crate::FastcgiScriptResolver;
#[cfg(feature = "core-bridge")]
use crate::FcgiRuntime;

/// Register module-owned runtime via caller-supplied core registration fn (CLI/tests).
#[cfg(feature = "core-bridge")]
pub fn register_with_core<F>(
    registration: FcgiRuntimeRegistration,
    register: F,
) -> Result<(), FcgiRegisterError>
where
    F: FnOnce(Arc<dyn FcgiDispatchService>) -> Result<(), FcgiRegisterError>,
{
    register(Arc::new(FcgiRuntime::new(registration)?))
}

/// Register module-owned FastCGI script resolver via module-api register-once slot.
#[cfg(feature = "core-bridge")]
pub fn register_script_resolver() -> Result<(), FastcgiScriptResolverRegisterError> {
    register_fastcgi_script_resolver(
        Arc::new(FastcgiScriptResolver::new()) as Arc<dyn FastcgiScriptResolutionService>
    )
}

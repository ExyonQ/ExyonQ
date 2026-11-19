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
//! NG-21: the committed `[tuning]` geometry reaches the resolver the wire paths
//! share, and dropping the keys restores the fallback.

use std::time::Duration;

#[test]
fn committed_geometry_reaches_the_header_timeout_resolver() {
    exyonq_module_api::runtime_tuning::publish(Some(400), Some(16_384));
    assert_eq!(
        exyonq_core::server::header_read_timeout(),
        Duration::from_millis(400),
        "the published [tuning].keepalive_timeout_ms must win"
    );
    exyonq_module_api::runtime_tuning::publish(Some(1234), Some(8192));
    assert_eq!(
        exyonq_core::server::header_read_timeout(),
        Duration::from_millis(1234),
        "a later commit republishes the value; nothing is cached past it"
    );
    exyonq_module_api::runtime_tuning::reset_for_tests();
}

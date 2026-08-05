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
//! FastCGI 501 aggregate observability (KD4.12) — shell + module runtime.

use crate::kernel_shell_metrics::fastcgi_http_501_total;

/// Total HTTP **501 responses on FastCGI contract routes** (observability contract A).
///
/// Sum of kernel shell counter ([`fastcgi_http_501_total`]) plus module runtime
/// [`exyonq_mod_fastcgi::fcgi_responses_501_total`] — at most one bucket increments per request.
pub fn fcgi_responses_501_total() -> u64 {
    fastcgi_http_501_total() + exyonq_mod_fastcgi::fcgi_responses_501_total()
}

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
//! Pinned generation view — no full [`crate::server::state::ServerState`] exposure.

/// Minimal pinned snapshot for wire planning and handoff (PS1C).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationView {
    pub generation: u64,
    pub modules_enabled: bool,
    pub site_static_slot: Option<u32>,
}

impl GenerationView {
    pub const fn pinned(
        generation: u64,
        modules_enabled: bool,
        site_static_slot: Option<u32>,
    ) -> Self {
        Self {
            generation,
            modules_enabled,
            site_static_slot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps1c_generation_view_is_copy_no_arc() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<GenerationView>();
    }
}

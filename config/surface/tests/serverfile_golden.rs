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
#[cfg(test)]
mod serverfile_golden {
    use exyonq_config_surface::{compile_serverfile, CompileOptions};
    use std::path::PathBuf;

    #[test]
    fn minimal_exy_matches_expected_toml() {
        let base =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/serverfile");
        let input = std::fs::read_to_string(base.join("minimal.exy")).unwrap();
        let expected = std::fs::read_to_string(base.join("minimal.expected.toml")).unwrap();
        let compiled = compile_serverfile(&input, CompileOptions::default()).unwrap();
        assert_eq!(compiled.trim(), expected.trim());
    }
}

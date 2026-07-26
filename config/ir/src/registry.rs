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
//! Static module directive registry (IR v2 extension point).

/// Describes a module section in IR v1 (fixed registry).
#[derive(Debug, Clone, Copy)]
pub struct ModuleDirectiveSpec {
    pub name: &'static str,
    pub ir_section: &'static str,
}

pub const MODULE_DIRECTIVES_V1: &[ModuleDirectiveSpec] = &[
    ModuleDirectiveSpec {
        name: "metrics",
        ir_section: "modules.metrics",
    },
    ModuleDirectiveSpec {
        name: "compression",
        ir_section: "modules.compression",
    },
    ModuleDirectiveSpec {
        name: "ratelimit",
        ir_section: "modules.ratelimit",
    },
];

pub fn lookup_module(name: &str) -> Option<&'static ModuleDirectiveSpec> {
    MODULE_DIRECTIVES_V1.iter().find(|d| d.name == name)
}

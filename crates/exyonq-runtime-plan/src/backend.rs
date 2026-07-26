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
//! Phase 0 backend dispatch types (ADR-029 PR-4).
//!
//! Compile-time binding table only — no executors, no hot-path wiring.

/// Dense index into [`BackendTable`] (ADR-029).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct BackendId(u32);

impl BackendId {
    /// Sentinel for unresolved bindings.
    pub const INVALID: BackendId = BackendId(u32::MAX);

    #[inline]
    pub fn from_index(index: u32) -> Self {
        Self(index)
    }

    #[inline]
    pub fn index(self) -> u32 {
        self.0
    }

    #[inline]
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Plan-time backend variant (ADR-029).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backend {
    Static {
        root_slot: u32,
    },
    Proxy {
        cluster_id: u32,
    },
    /// Contract-only until Plan 08 gate — must not execute before signoff.
    Fastcgi {
        pool_id: u32,
    },
    /// Contract-only until module executor wiring.
    Module {
        hook_id: u32,
    },
}

impl Backend {
    /// True for variants that have no runtime executor.
    #[inline]
    pub fn is_contract_only(&self) -> bool {
        matches!(self, Backend::Fastcgi { .. } | Backend::Module { .. })
    }
}

/// Sole handler binding table (ADR-029 HP-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendTable {
    backends: Box<[Backend]>,
}

impl BackendTable {
    #[inline]
    pub fn empty() -> Self {
        Self {
            backends: Box::new([]),
        }
    }

    /// Build a dense table at compile time.
    pub fn from_backends(backends: Vec<Backend>) -> Self {
        Self {
            backends: backends.into_boxed_slice(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    #[inline]
    pub fn get(&self, id: BackendId) -> Option<&Backend> {
        self.backends.get(id.as_usize())
    }
}

impl Default for BackendTable {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_id_index_and_as_usize() {
        let id = BackendId::from_index(3);
        assert_eq!(id.index(), 3);
        assert_eq!(id.as_usize(), 3);
        assert_ne!(BackendId::INVALID.index(), 3);
    }

    #[test]
    fn backend_table_empty_and_default() {
        let empty = BackendTable::empty();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(BackendTable::default().len(), 0);
    }

    #[test]
    fn backend_table_get_in_bounds_and_out_of_bounds() {
        let table = BackendTable::from_backends(vec![
            Backend::Static { root_slot: 0 },
            Backend::Proxy { cluster_id: 7 },
        ]);
        assert_eq!(table.len(), 2);
        assert!(matches!(
            table.get(BackendId::from_index(0)),
            Some(Backend::Static { root_slot: 0 })
        ));
        assert!(matches!(
            table.get(BackendId::from_index(1)),
            Some(Backend::Proxy { cluster_id: 7 })
        ));
        assert!(table.get(BackendId::from_index(2)).is_none());
        assert!(table.get(BackendId::INVALID).is_none());
    }

    #[test]
    fn backend_is_contract_only() {
        assert!(!Backend::Static { root_slot: 0 }.is_contract_only());
        assert!(!Backend::Proxy { cluster_id: 1 }.is_contract_only());
        assert!(Backend::Fastcgi { pool_id: 0 }.is_contract_only());
        assert!(Backend::Module { hook_id: 0 }.is_contract_only());
    }
}

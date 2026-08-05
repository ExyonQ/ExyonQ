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
//! Phase 0 route match outcomes (ADR-029 PR-4).

use crate::backend::BackendId;

/// Outcome of route matching against a compiled plan (ADR-029).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteDecision {
    NotFound,
    Redirect { status: u16, location: String },
    Rewrite { path: String },
    Backend(BackendId),
}

impl RouteDecision {
    #[inline]
    pub fn backend_id(&self) -> Option<BackendId> {
        match self {
            RouteDecision::Backend(id) => Some(*id),
            _ => None,
        }
    }

    #[inline]
    pub fn is_backend(&self) -> bool {
        matches!(self, RouteDecision::Backend(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, BackendTable};

    #[test]
    fn backend_decision_exposes_id() {
        let id = BackendId::from_index(2);
        let decision = RouteDecision::Backend(id);
        assert_eq!(decision.backend_id(), Some(id));
        assert!(decision.is_backend());
    }

    #[test]
    fn non_backend_decisions_return_none() {
        assert_eq!(RouteDecision::NotFound.backend_id(), None);
        assert_eq!(
            RouteDecision::Redirect {
                status: 301,
                location: "/x".into(),
            }
            .backend_id(),
            None
        );
        assert_eq!(
            RouteDecision::Rewrite {
                path: "/index.html".into(),
            }
            .backend_id(),
            None
        );
        assert!(!RouteDecision::NotFound.is_backend());
    }

    #[test]
    fn decision_and_table_resolve_backend_variant() {
        let table = BackendTable::from_backends(vec![
            Backend::Static { root_slot: 0 },
            Backend::Proxy { cluster_id: 4 },
        ]);
        let decision = RouteDecision::Backend(BackendId::from_index(1));
        let id = decision.backend_id().expect("backend id");
        let backend = table.get(id).expect("in bounds");
        assert!(matches!(backend, Backend::Proxy { cluster_id: 4 }));
    }
}

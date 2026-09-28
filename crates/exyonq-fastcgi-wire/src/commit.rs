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
//! Explicit FastCGI wire commit stages for safe-retry decisions.

/// How far a FastCGI exchange has progressed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommitStage {
    /// No request bytes written for this attempt.
    NotStarted = 0,
    /// At least one byte of BEGIN_REQUEST was written.
    BeginCommitted = 1,
    /// PARAMS frames fully written (BEGIN already committed).
    ParamsCommitted = 2,
    /// STDIN (including empty terminator) fully written.
    BodyCommitted = 3,
    /// At least one response record byte observed.
    ResponseStarted = 4,
}

impl CommitStage {
    /// Safe retry is allowed only before any request bytes were committed.
    pub fn allows_safe_retry(self) -> bool {
        self == Self::NotStarted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_not_started_allows_retry() {
        assert!(CommitStage::NotStarted.allows_safe_retry());
        assert!(!CommitStage::BeginCommitted.allows_safe_retry());
        assert!(!CommitStage::ParamsCommitted.allows_safe_retry());
        assert!(!CommitStage::BodyCommitted.allows_safe_retry());
        assert!(!CommitStage::ResponseStarted.allows_safe_retry());
    }
}

//! Core filesystem probes for htaccess runtime (neutral I/O; module owns policy).

use exyonq_module_api::fcgi_script_resolver::{
    fastcgi_script_resolver, FastcgiScriptResolutionOutcome, FastcgiScriptResolutionPurpose,
    FastcgiScriptResolutionRequest,
};
use exyonq_module_api::htaccess_runtime::{HtaccessResourceKind, HtaccessRuntimeProbes};
use std::path::Path;

pub struct CoreHtaccessProbes;

impl HtaccessRuntimeProbes for CoreHtaccessProbes {
    fn probe_static_directory_index(
        &self,
        root_slot: u32,
        dir_uri: &str,
        candidates: &[String],
    ) -> Option<String> {
        crate::execute_backend::probe_static_index(root_slot, dir_uri, candidates)
    }

    fn probe_fcgi_directory_index_script(
        &self,
        document_root: &str,
        _dir_uri: &str,
        candidate_uri: &str,
    ) -> bool {
        let Some(resolver) = fastcgi_script_resolver() else {
            return false;
        };
        matches!(
            resolver.resolve(&FastcgiScriptResolutionRequest {
                uri_path: candidate_uri.to_string(),
                pool_document_root: Some(Path::new(document_root).to_path_buf()),
                generation: 0,
                purpose: FastcgiScriptResolutionPurpose::Probe,
            }),
            FastcgiScriptResolutionOutcome::Resolved { .. }
        )
    }

    fn probe_fcgi_directory_index_static_file(
        &self,
        document_root: &str,
        child_uri: &str,
    ) -> Option<String> {
        // Probe as a dispatch-like resolution; if it resolves, treat it as a file hit.
        let resolver = fastcgi_script_resolver()?;
        match resolver.resolve(&FastcgiScriptResolutionRequest {
            uri_path: child_uri.to_string(),
            pool_document_root: Some(Path::new(document_root).to_path_buf()),
            generation: 0,
            purpose: FastcgiScriptResolutionPurpose::Probe,
        }) {
            FastcgiScriptResolutionOutcome::Resolved {
                script_filename, ..
            } => Some(script_filename),
            _ => None,
        }
    }

    fn probe_requested_resource(
        &self,
        document_root: &str,
        uri_path: &str,
    ) -> Result<HtaccessResourceKind, ()> {
        // Minimal, neutral classification: file hit means "File"; anything else is "Missing".
        // DirectoryIndex + front-controller policy stays in the module.
        let resolver = fastcgi_script_resolver().ok_or(())?;
        match resolver.resolve(&FastcgiScriptResolutionRequest {
            uri_path: uri_path.to_string(),
            pool_document_root: Some(Path::new(document_root).to_path_buf()),
            generation: 0,
            purpose: FastcgiScriptResolutionPurpose::Probe,
        }) {
            FastcgiScriptResolutionOutcome::Resolved { .. } => Ok(HtaccessResourceKind::File),
            FastcgiScriptResolutionOutcome::NotFound => Ok(HtaccessResourceKind::Missing),
            _ => Err(()),
        }
    }

    fn fcgi_script_resolvable(&self, document_root: &str, uri_path: &str) -> bool {
        let Some(resolver) = fastcgi_script_resolver() else {
            return false;
        };
        matches!(
            resolver.resolve(&FastcgiScriptResolutionRequest {
                uri_path: uri_path.to_string(),
                pool_document_root: Some(Path::new(document_root).to_path_buf()),
                generation: 0,
                purpose: FastcgiScriptResolutionPurpose::Probe,
            }),
            FastcgiScriptResolutionOutcome::Resolved { .. }
        )
    }
}

use crate::error::CtrlError;
use crate::mutate::{self, MutationSummary};
use crate::revision::{self, RevisionState};
use crate::types::{
    CapabilitySet, Health, Revision, RuntimeConfig, Stats, Vhost, VhostCreate, VhostUpstream,
};
use exyonq_config_ir::AppConfig;
use exyonq_module_api::kernel_control::KernelControlPort;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;

#[derive(Clone)]
pub struct ControlService {
    inner: Arc<ControlServiceInner>,
}

struct ControlServiceInner {
    port: Arc<dyn KernelControlPort>,
    config_path: PathBuf,
    mutation_lock: Mutex<()>,
}

impl ControlService {
    pub fn new(port: Arc<dyn KernelControlPort>, config_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(ControlServiceInner {
                port,
                config_path,
                mutation_lock: Mutex::new(()),
            }),
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.inner.config_path
    }

    pub fn current_revision(&self) -> Result<RevisionState, CtrlError> {
        revision::load(&self.inner.config_path)
    }

    pub fn health(&self) -> Result<Health, CtrlError> {
        let rev = self.current_revision()?;
        let outcome = self.inner.port.read_status(false);
        Ok(Health {
            status: if outcome.ok && !outcome.snapshot.draining {
                "ok".to_owned()
            } else {
                "not_ready".to_owned()
            },
            generation: outcome.snapshot.generation,
            revision: rev.revision,
            draining: outcome.snapshot.draining,
            reload_in_progress: outcome.snapshot.reload_in_progress,
        })
    }

    pub fn stats(&self) -> Result<Stats, CtrlError> {
        let rev = self.current_revision()?;
        let outcome = self.inner.port.read_status(true);
        Ok(Stats {
            generation: outcome.snapshot.generation,
            revision: rev.revision,
            uptime_s: outcome.snapshot.uptime_s,
            active_connections: outcome.snapshot.active_connections,
            draining: outcome.snapshot.draining,
            reload_in_progress: outcome.snapshot.reload_in_progress,
            fingerprint: Some(outcome.snapshot.fingerprint),
            version: outcome.snapshot.version,
        })
    }

    pub fn runtime_config(&self) -> Result<RuntimeConfig, CtrlError> {
        let rev = self.current_revision()?;
        let outcome = self.inner.port.read_status(true);
        let config = exyonq_config_merge::load_with_includes(&self.inner.config_path)
            .map_err(|err| CtrlError::BadRequest(err.to_string()))?;
        Ok(RuntimeConfig {
            revision: rev.revision,
            generation: outcome.snapshot.generation,
            fingerprint: Some(outcome.snapshot.fingerprint),
            vhosts: vhosts_from_config(&config, rev.revision, outcome.snapshot.generation),
            capabilities: CapabilitySet::default(),
        })
    }

    pub async fn create_vhost(
        &self,
        input: VhostCreate,
        if_match: Option<&str>,
    ) -> Result<Vhost, CtrlError> {
        let service = self.clone();
        let if_match = if_match.map(ToOwned::to_owned);
        tokio::spawn(async move { service.create_vhost_inner(input, if_match.as_deref()).await })
            .await
            .map_err(|err| CtrlError::ServiceUnavailable(err.to_string()))?
    }

    async fn create_vhost_inner(
        &self,
        input: VhostCreate,
        if_match: Option<&str>,
    ) -> Result<Vhost, CtrlError> {
        let _guard = self.inner.mutation_lock.lock().await;
        let current = self.current_revision()?;
        if !revision::matches_current(if_match, current, false) {
            return Err(CtrlError::PreconditionFailed);
        }
        let staged = mutate::stage_create(&self.inner.config_path, &input)?;
        let outcome = self
            .inner
            .port
            .request_reload(&self.inner.config_path)
            .await;
        if !outcome.ok {
            self.restore_last_good(&staged.backup_path).await?;
            return Err(reload_error(outcome.code, outcome.error));
        }
        let next = match revision::bump(&self.inner.config_path) {
            Ok(next) => next,
            Err(err) => {
                self.restore_last_good(&staged.backup_path).await?;
                return Err(err);
            }
        };
        Ok(vhost_from_summary(
            staged.summary,
            next.revision,
            outcome.snapshot.generation,
        ))
    }

    pub async fn delete_vhost(
        &self,
        domain: &str,
        if_match: Option<&str>,
    ) -> Result<Revision, CtrlError> {
        let service = self.clone();
        let domain = domain.to_owned();
        let if_match = if_match.map(ToOwned::to_owned);
        tokio::spawn(async move {
            service
                .delete_vhost_inner(&domain, if_match.as_deref())
                .await
        })
        .await
        .map_err(|err| CtrlError::ServiceUnavailable(err.to_string()))?
    }

    async fn delete_vhost_inner(
        &self,
        domain: &str,
        if_match: Option<&str>,
    ) -> Result<Revision, CtrlError> {
        let _guard = self.inner.mutation_lock.lock().await;
        let current = self.current_revision()?;
        if !revision::matches_current(if_match, current, true) {
            return Err(CtrlError::PreconditionFailed);
        }
        let staged = mutate::stage_delete(&self.inner.config_path, domain)?;
        let outcome = self
            .inner
            .port
            .request_reload(&self.inner.config_path)
            .await;
        if !outcome.ok {
            self.restore_last_good(&staged.backup_path).await?;
            return Err(reload_error(outcome.code, outcome.error));
        }
        let next = match revision::bump(&self.inner.config_path) {
            Ok(next) => next,
            Err(err) => {
                self.restore_last_good(&staged.backup_path).await?;
                return Err(err);
            }
        };
        Ok(Revision {
            revision: next.revision,
            generation: outcome.snapshot.generation,
        })
    }

    async fn restore_last_good(&self, backup_path: &Path) -> Result<(), CtrlError> {
        if let Err(err) = mutate::restore_backup(&self.inner.config_path, backup_path) {
            warn!(%err, "control api failed to restore config backup after reload rejection");
            return Err(CtrlError::ServiceUnavailable(
                "reload rejected and backup restore failed".to_owned(),
            ));
        }
        let restore = self
            .inner
            .port
            .request_reload(&self.inner.config_path)
            .await;
        if !restore.ok {
            warn!(
                code = restore.code.as_deref().unwrap_or(""),
                "control api backup restore reload was rejected"
            );
            return Err(CtrlError::ServiceUnavailable(
                "reload rejected and backup restore reload failed".to_owned(),
            ));
        }
        Ok(())
    }
}

fn reload_error(code: Option<String>, error: Option<String>) -> CtrlError {
    CtrlError::ReloadRejected {
        code,
        message: error.unwrap_or_else(|| "reload rejected".to_owned()),
    }
}

fn vhost_from_summary(summary: MutationSummary, revision: u64, generation: u64) -> Vhost {
    Vhost {
        domain: summary.domain,
        listen: Some(summary.listen),
        path: summary.path,
        root: summary.root,
        upstream: summary.upstream,
        revision,
        generation,
    }
}

fn vhosts_from_config(config: &AppConfig, revision: u64, generation: u64) -> Vec<Vhost> {
    let mut out = Vec::new();
    for server in &config.servers {
        for domain in server.server_name_list() {
            let route = resolve_route_for_domain(config, server, &domain);
            let upstream = route
                .and_then(|route| route.upstream.as_ref())
                .and_then(|name| config.upstreams.get(name))
                .map(|upstream| VhostUpstream {
                    target: upstream.target.clone(),
                    timeout_ms: upstream.timeout_ms,
                });
            out.push(Vhost {
                domain,
                listen: Some(server.listen.clone()),
                path: route
                    .map(|route| route.r#match.path.clone())
                    .unwrap_or_else(|| "/".to_owned()),
                root: route
                    .and_then(|route| route.root.as_ref())
                    .map(|root| root.display().to_string()),
                upstream,
                revision,
                generation,
            });
        }
    }
    out
}

fn resolve_route_for_domain<'a>(
    config: &'a AppConfig,
    server: &exyonq_config_ir::ServerConfig,
    domain: &str,
) -> Option<&'a exyonq_config_ir::RouteConfig> {
    let control_name = mutate::route_name(domain);
    if let Some(route) = config
        .routes
        .iter()
        .find(|route| route.name == control_name)
    {
        return Some(route);
    }
    for name in &server.routes {
        if let Some(route) = config.routes.iter().find(|route| &route.name == name) {
            if route.r#match.host.as_deref() == Some(domain) {
                return Some(route);
            }
        }
    }
    server
        .routes
        .first()
        .and_then(|name| config.routes.iter().find(|route| &route.name == name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use exyonq_module_api::kernel_control::{KernelStatusSnapshot, OpsCommand, OpsCommandOutcome};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::sync::Notify;

    struct StubPort {
        generation: AtomicU64,
        hold_reload: AtomicBool,
        poison_revision_tmp_after_reload: AtomicBool,
        reload_started: Notify,
        release_reload: Notify,
        reload_completed: Notify,
    }

    impl StubPort {
        fn new(hold_reload: bool) -> Self {
            Self {
                generation: AtomicU64::new(0),
                hold_reload: AtomicBool::new(hold_reload),
                poison_revision_tmp_after_reload: AtomicBool::new(false),
                reload_started: Notify::new(),
                release_reload: Notify::new(),
                reload_completed: Notify::new(),
            }
        }

        fn snapshot(&self) -> KernelStatusSnapshot {
            KernelStatusSnapshot {
                generation: self.generation.load(Ordering::SeqCst),
                fingerprint: "test-fp".to_owned(),
                uptime_s: 0,
                active_connections: 0,
                draining: false,
                version: Some("test".to_owned()),
                reload_in_progress: false,
            }
        }
    }

    #[async_trait]
    impl KernelControlPort for StubPort {
        async fn request_reload(&self, config_path: &Path) -> OpsCommandOutcome {
            self.reload_started.notify_waiters();
            if self.hold_reload.swap(false, Ordering::SeqCst) {
                self.release_reload.notified().await;
            }
            self.generation.fetch_add(1, Ordering::SeqCst);
            if self
                .poison_revision_tmp_after_reload
                .swap(false, Ordering::SeqCst)
            {
                std::fs::create_dir_all(
                    revision::state_path(config_path).with_extension("json.tmp"),
                )
                .expect("poison revision temp path");
            }
            self.reload_completed.notify_waiters();
            OpsCommandOutcome::from_parts(true, OpsCommand::Reload, self.snapshot(), None, None)
        }

        fn request_drain(&self) -> OpsCommandOutcome {
            OpsCommandOutcome::from_parts(true, OpsCommand::Drain, self.snapshot(), None, None)
        }

        fn request_shutdown(&self) -> OpsCommandOutcome {
            OpsCommandOutcome::from_parts(true, OpsCommand::Shutdown, self.snapshot(), None, None)
        }

        fn read_status(&self, include_version: bool) -> OpsCommandOutcome {
            let mut snapshot = self.snapshot();
            if !include_version {
                snapshot.version = None;
            }
            OpsCommandOutcome::from_parts(true, OpsCommand::Status, snapshot, None, None)
        }
    }

    fn temp_config_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "exyonq-control-api-{name}-{}-{stamp}.toml",
            std::process::id()
        ))
    }

    fn write_base_config(path: &Path) {
        std::fs::write(
            path,
            r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "/srv/site"
"#,
        )
        .expect("write base config");
    }

    fn vhost(domain: &str) -> VhostCreate {
        VhostCreate {
            domain: domain.to_owned(),
            listen: None,
            path: "/".to_owned(),
            root: Some(format!("/srv/{domain}")),
            upstream: None,
        }
    }

    #[tokio::test]
    async fn concurrent_creates_are_serialized_across_stage_reload_and_revision() {
        let path = temp_config_path("serialized");
        write_base_config(&path);
        let port = Arc::new(StubPort::new(false));
        let service = ControlService::new(port, path.clone());

        let (first, second) = tokio::join!(
            service.create_vhost(vhost("a.example.test"), None),
            service.create_vhost(vhost("b.example.test"), None)
        );

        let mut revisions = [
            first.expect("first create").revision,
            second.expect("second create").revision,
        ];
        revisions.sort_unstable();
        assert_eq!(revisions, [1, 2]);
        let raw = std::fs::read_to_string(&path).expect("read config");
        assert!(raw.contains("a.example.test"));
        assert!(raw.contains("b.example.test"));
        assert_eq!(revision::load(&path).expect("revision").revision, 2);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(revision::state_path(&path));
    }

    #[tokio::test]
    async fn cancelled_request_does_not_cancel_staged_reload_transaction() {
        let path = temp_config_path("cancel");
        write_base_config(&path);
        let port = Arc::new(StubPort::new(true));
        let service = ControlService::new(port.clone(), path.clone());

        let task = tokio::spawn({
            let service = service.clone();
            async move {
                service
                    .create_vhost(vhost("cancel.example.test"), None)
                    .await
            }
        });
        port.reload_started.notified().await;
        task.abort();
        port.release_reload.notify_waiters();
        port.reload_completed.notified().await;

        let raw = std::fs::read_to_string(&path).expect("read config");
        assert!(raw.contains("cancel.example.test"));
        assert_eq!(revision::load(&path).expect("revision").revision, 1);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(revision::state_path(&path));
    }

    #[tokio::test]
    async fn revision_commit_failure_rolls_back_staged_config() {
        let path = temp_config_path("revision-fail");
        write_base_config(&path);
        let port = Arc::new(StubPort::new(false));
        port.poison_revision_tmp_after_reload
            .store(true, Ordering::SeqCst);
        let service = ControlService::new(port, path.clone());

        let result = service
            .create_vhost(vhost("revision-fail.example.test"), None)
            .await;

        assert!(result.is_err());
        let raw = std::fs::read_to_string(&path).expect("read config");
        assert!(!raw.contains("revision-fail.example.test"));
        assert_eq!(
            revision::load(&path)
                .expect("revision remains readable")
                .revision,
            0
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(revision::state_path(&path).with_extension("json.tmp"));
    }
}

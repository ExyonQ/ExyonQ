//! Debounced filesystem watcher (off request path).

use crate::compiler::{compile_vhost_overlay, CompileError};
use crate::limits::WATCHER_DEBOUNCE_MS;
use crate::store::OverlayPublisher;
use exyonq_module_api::RuntimePatchVhostOverlay;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct HtaccessSite {
    pub site_id: String,
    pub document_root: PathBuf,
}

pub fn spawn_watcher(
    sites: Vec<HtaccessSite>,
    publisher: Arc<OverlayPublisher>,
) -> anyhow::Result<()> {
    if sites.is_empty() {
        return Ok(());
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();
    let watch_roots: Vec<PathBuf> = {
        let mut roots: Vec<PathBuf> = sites.iter().map(|s| s.document_root.clone()).collect();
        roots.sort();
        roots.dedup();
        roots
    };

    for root in watch_roots {
        let tx = tx.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Create(_)
                        | EventKind::Modify(_)
                        | EventKind::Remove(_)
                        | EventKind::Any => {
                            let _ = tx.send(());
                        }
                        _ => {}
                    }
                }
            },
            notify::Config::default(),
        )?;
        watcher.watch(root.as_path(), RecursiveMode::Recursive)?;
        std::mem::forget(watcher);
    }

    let sites_for_task = sites.clone();
    let publisher_for_task = Arc::clone(&publisher);
    tokio::spawn(async move {
        let debounce = Duration::from_millis(WATCHER_DEBOUNCE_MS);
        while rx.recv().await.is_some() {
            while rx.try_recv().is_ok() {}
            tokio::time::sleep(debounce).await;
            recompile_all(&sites_for_task, &publisher_for_task);
        }
    });

    recompile_all(&sites, &publisher);
    Ok(())
}

fn recompile_all(sites: &[HtaccessSite], publisher: &Arc<OverlayPublisher>) {
    for site in sites {
        match compile_and_publish(site, publisher) {
            Ok(gen) => info!(
                site = %site.site_id,
                generation = gen,
                "htaccess overlay published"
            ),
            Err(err) => {
                publisher.record_compile_failure();
                warn!(site = %site.site_id, error = %err, "htaccess compile failed; prior overlay retained");
            }
        }
    }
}

pub fn compile_and_publish(
    site: &HtaccessSite,
    publisher: &Arc<OverlayPublisher>,
) -> Result<u64, CompileError> {
    let next_overlay_gen = publisher.overlay_generation() + 1;
    let output = compile_vhost_overlay(&site.site_id, &site.document_root, next_overlay_gen)?;
    publisher.record_files_compiled(1);
    publisher.record_unsupported(output.report.parsed_only as u64 + output.report.unknown as u64);
    if !output.report.errors.is_empty() {
        return Err(CompileError::Message(output.report.errors.join("; ")));
    }
    let patch = RuntimePatchVhostOverlay {
        site_id: site.site_id.clone(),
        plan_generation: publisher.plan_generation(),
        overlay: output.overlay,
    };
    publisher
        .publish(patch)
        .map_err(|_| CompileError::Message("publish rejected".into()))
}

pub fn initial_compile(site: &HtaccessSite, publisher: &Arc<OverlayPublisher>) {
    if let Err(err) = compile_and_publish(site, publisher) {
        publisher.record_compile_failure();
        tracing::error!(site = %site.site_id, error = %err, "initial htaccess compile failed");
    }
}

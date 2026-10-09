//! Debounced filesystem watcher (off request path).

use crate::compiler::{compile_vhost_overlay, CompileError};
use crate::limits::WATCHER_DEBOUNCE_MS;
use crate::store::OverlayPublisher;
use exyonq_module_api::RuntimePatchVhostOverlay;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
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
                            if event_touches_htaccess(&event.paths) {
                                let _ = tx.send(());
                            }
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
        let before = publisher.overlay_generation();
        match compile_and_publish(site, publisher) {
            Ok(gen) if gen > before => info!(
                site = %site.site_id,
                generation = gen,
                "htaccess overlay published"
            ),
            Ok(_) => {}
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
    let fingerprint = htaccess_fingerprint(&site.document_root);
    let key = format!("{}|{}", site.site_id, site.document_root.display());
    if published_fingerprints()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(&key)
        .is_some_and(|previous| *previous == fingerprint)
    {
        return Ok(publisher.overlay_generation());
    }
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
    let generation = publisher
        .publish(patch)
        .map_err(|_| CompileError::Message("publish rejected".into()))?;
    published_fingerprints()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(key, fingerprint);
    Ok(generation)
}

/// True when an event names a `.htaccess` file. Empty paths are ignored so a
/// SQLite or session write cannot republish the site.
pub(crate) fn event_touches_htaccess(paths: &[PathBuf]) -> bool {
    paths
        .iter()
        .any(|path| path.file_name().is_some_and(|name| name == ".htaccess"))
}

fn published_fingerprints() -> &'static Mutex<HashMap<String, u64>> {
    static SLOT: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn htaccess_fingerprint(root: &Path) -> u64 {
    let mut files = Vec::new();
    collect_htaccess_files(root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = DefaultHasher::new();
    for (path, bytes) in &files {
        path.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    hasher.finish()
}

fn collect_htaccess_files(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect_htaccess_files(&path, out);
            continue;
        }
        if entry.file_name() == ".htaccess" {
            if let Ok(bytes) = std::fs::read(&path) {
                out.push((path, bytes));
            }
        }
    }
}

pub fn initial_compile(site: &HtaccessSite, publisher: &Arc<OverlayPublisher>) {
    if let Err(err) = compile_and_publish(site, publisher) {
        publisher.record_compile_failure();
        tracing::error!(site = %site.site_id, error = %err, "initial htaccess compile failed");
    }
}

#[cfg(test)]
mod tests {
    use super::event_touches_htaccess;
    use std::path::PathBuf;

    #[test]
    fn only_an_htaccess_path_is_recompiled() {
        assert!(!event_touches_htaccess(&[]));
        assert!(!event_touches_htaccess(&[PathBuf::from(
            "/var/www/wp-content/database/.ht.sqlite"
        )]));
        assert!(event_touches_htaccess(&[PathBuf::from("/var/www/.htaccess")]));
        assert!(event_touches_htaccess(&[
            PathBuf::from("/var/www/wp-content/database/.ht.sqlite"),
            PathBuf::from("/var/www/wp-admin/.htaccess"),
        ]));
    }
}

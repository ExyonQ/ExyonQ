//! WC7C lab hooks — register purge port builders without Redis types in core.
//!
//! Activated only when composition (cli) installs a builder under `EXYONQ_L2_COORD_LAB=1`.

use crate::reload::SharedServerState;
use exyonq_module_api::cache_purge::CachePurgePort;
use std::sync::{Arc, OnceLock, RwLock};

type PurgeBuilder = Arc<dyn Fn(SharedServerState) -> Arc<dyn CachePurgePort> + Send + Sync>;
type SubscriberStarter = Arc<dyn Fn(SharedServerState) + Send + Sync>;

static PURGE_BUILDER: OnceLock<RwLock<Option<PurgeBuilder>>> = OnceLock::new();
static SUBSCRIBER_STARTER: OnceLock<RwLock<Option<SubscriberStarter>>> = OnceLock::new();

fn purge_slot() -> &'static RwLock<Option<PurgeBuilder>> {
    PURGE_BUILDER.get_or_init(|| RwLock::new(None))
}

fn sub_slot() -> &'static RwLock<Option<SubscriberStarter>> {
    SUBSCRIBER_STARTER.get_or_init(|| RwLock::new(None))
}

/// Install lab purge-port builder (cli composition). Replaces any previous.
pub fn install_lab_purge_port_builder(builder: PurgeBuilder) {
    if let Ok(mut g) = purge_slot().write() {
        *g = Some(builder);
    }
}

/// Install lab subscriber apply-loop starter.
pub fn install_lab_subscriber_starter(starter: SubscriberStarter) {
    if let Ok(mut g) = sub_slot().write() {
        *g = Some(starter);
    }
}

pub(crate) fn build_lab_or_default_purge_port(
    shared: SharedServerState,
) -> Arc<dyn CachePurgePort> {
    if let Ok(g) = purge_slot().read() {
        if let Some(b) = g.as_ref() {
            return b(shared);
        }
    }
    crate::cache_purge_port::purge_port(shared)
}

pub(crate) fn start_lab_subscriber_if_any(shared: SharedServerState) {
    if let Ok(g) = sub_slot().read() {
        if let Some(s) = g.as_ref() {
            s(shared);
        }
    }
}

// Cache only when public [[cache_policy]] is present in RuntimePlan.
fn maybe_cache(plan: &Plan) {
    if plan.has_cache_policy() {
        cache_store();
    }
}

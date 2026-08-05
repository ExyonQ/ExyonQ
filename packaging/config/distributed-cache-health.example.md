# Health-check example (dataplane vs coordination)
#
# Dataplane readiness: existing ExyonQ /health (or systemd readiness) — MUST remain
# independent of Redis coordination state.
#
# Coordination health (operator-facing, optional scrape of process metrics):
#   l2_redis_provider_degraded
#   CoordinationProviderHealth labels: DISABLED|CONNECTING|HEALTHY|DEGRADED|RECONCILING
#
# Example probe (shell): treat Redis down as coordination degraded, never as traffic fail.
#
# curl -fsS http://127.0.0.1:8080/health || exit 1
# # Optional: alert if l2_redis_provider_degraded==1 for >5m — do not kill pod for this alone.

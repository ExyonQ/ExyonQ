# AUDITOR must-pass: $MODE as a campaign selector is not smoke execution.
MODE=probe
cat >>"$EV_DIR/${target}.${MODE}.meta.env" <<EOF
MODE=$MODE
EOF
echo "TARGET=$t MODE=$mode"

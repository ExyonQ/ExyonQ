(module
  (import "env" "monotonic_now_ns" (func $now (result i64)))
  (memory (export "memory") 2)
  (func (export "exyonq_on_request_headers")
    (param i64 i32 i32 i32 i32) (result i32)
    i32.const 0)
)

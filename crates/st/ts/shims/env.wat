;; Extism's own logging and HTTP imports of js-pdk, linked in by `st build`: they are outside the
;; screener ABI. The TypeScript PDK routes console and fetch to the host functions log and http,
;; so only a direct Http.request call can reach http_request, and it traps.
(module
  (func (export "get_log_level") (result i32) (i32.const 2147483647))
  (func (export "log_trace") (param i64))
  (func (export "log_debug") (param i64))
  (func (export "log_info") (param i64))
  (func (export "log_warn") (param i64))
  (func (export "log_error") (param i64))
  (func (export "http_request") (param i64 i64) (result i64) (unreachable))
  (func (export "http_status_code") (result i32) (i32.const 0))
  (func (export "http_headers") (result i64) (i64.const 0)))

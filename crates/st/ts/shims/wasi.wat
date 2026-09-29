;; wasi_snapshot_preview1 for js-pdk modules, linked in by `st build` (wasm-merge): the clock comes
;; from the host function now_ms, randomness from splitmix64 seeded by now_ms; there are no files,
;; no environment and no process to exit.
(module
  (import "main" "memory" (memory 0))
  (import "extism:host/user" "now_ms" (func $now_ms (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "free" (func $free (param i64)))
  (global $rng (mut i64) (i64.const 0))

  ;; {"ok":1790699566515} -> 1790699566515
  (func $ms (result i64)
    (local $off i64) (local $len i64) (local $i i64) (local $c i32) (local $v i64) (local $seen i32)
    (local.set $off (call $now_ms (i64.const 0)))
    (local.set $len (call $length (local.get $off)))
    (block $done
      (loop $next
        (br_if $done (i64.ge_u (local.get $i) (local.get $len)))
        (local.set $c (call $load_u8 (i64.add (local.get $off) (local.get $i))))
        (if (i32.lt_u (i32.sub (local.get $c) (i32.const 48)) (i32.const 10))
          (then
            (local.set $v (i64.add (i64.mul (local.get $v) (i64.const 10))
                                   (i64.extend_i32_u (i32.sub (local.get $c) (i32.const 48)))))
            (local.set $seen (i32.const 1)))
          (else (br_if $done (local.get $seen))))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $next)))
    (call $free (local.get $off))
    (local.get $v))

  (func (export "clock_time_get") (param $id i32) (param $precision i64) (param $out i32) (result i32)
    (i64.store (local.get $out) (i64.mul (call $ms) (i64.const 1000000)))
    (i32.const 0))

  (func $splitmix (result i64) (local $z i64)
    (if (i64.eqz (global.get $rng)) (then (global.set $rng (call $ms))))
    (global.set $rng (i64.add (global.get $rng) (i64.const 0x9E3779B97F4A7C15)))
    (local.set $z (global.get $rng))
    (local.set $z (i64.mul (i64.xor (local.get $z) (i64.shr_u (local.get $z) (i64.const 30))) (i64.const 0xBF58476D1CE4E5B9)))
    (local.set $z (i64.mul (i64.xor (local.get $z) (i64.shr_u (local.get $z) (i64.const 27))) (i64.const 0x94D049BB133111EB)))
    (i64.xor (local.get $z) (i64.shr_u (local.get $z) (i64.const 31))))

  (func (export "random_get") (param $buf i32) (param $len i32) (result i32) (local $i i32)
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (i32.store8 (i32.add (local.get $buf) (local.get $i)) (i32.wrap_i64 (call $splitmix)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $next)))
    (i32.const 0))

  (func (export "environ_sizes_get") (param $count i32) (param $size i32) (result i32)
    (i32.store (local.get $count) (i32.const 0))
    (i32.store (local.get $size) (i32.const 0))
    (i32.const 0))
  (func (export "environ_get") (param i32 i32) (result i32) (i32.const 0))
  ;; 8 = EBADF: no descriptors and no preopens; stdout/stderr go nowhere (console goes to log)
  (func (export "fd_close") (param i32) (result i32) (i32.const 8))
  (func (export "fd_fdstat_get") (param i32 i32) (result i32) (i32.const 8))
  (func (export "fd_prestat_get") (param i32 i32) (result i32) (i32.const 8))
  (func (export "fd_prestat_dir_name") (param i32 i32 i32) (result i32) (i32.const 8))
  (func (export "fd_read") (param i32 i32 i32 i32) (result i32) (i32.const 8))
  (func (export "fd_seek") (param i32 i64 i32 i32) (result i32) (i32.const 8))
  (func (export "fd_write") (param i32 i32 i32 i32) (result i32) (i32.const 8))
  (func (export "proc_exit") (param i32) (unreachable)))

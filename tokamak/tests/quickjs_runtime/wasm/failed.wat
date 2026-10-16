(module
  (import "env" "table" (table 1 funcref))
  (import "env" "value" (func $value (result i32)))
  (func $escaped (result i32) call $value)
  (elem (i32.const 0) $escaped)
  (elem (i32.const 1) $escaped)
)

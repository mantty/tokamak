(module
  (func (export "swizzle") (result i32) v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 v128.const i8x16 7 7 7 7 7 7 7 7 7 7 7 7 7 7 7 7 i8x16.relaxed_swizzle i8x16.extract_lane_u 0)
  (func (export "trunc_f32x4_s") (result i32) v128.const f32x4 42.75 42.75 42.75 42.75 i32x4.relaxed_trunc_f32x4_s i32x4.extract_lane 0)
  (func (export "trunc_f32x4_u") (result i32) v128.const f32x4 42.75 42.75 42.75 42.75 i32x4.relaxed_trunc_f32x4_u i32x4.extract_lane 0)
  (func (export "f32x4_madd") (result f32) v128.const f32x4 3 3 3 3 v128.const f32x4 4 4 4 4 v128.const f32x4 30 30 30 30 f32x4.relaxed_madd f32x4.extract_lane 0)
  (func (export "f32x4_nmadd") (result f32) v128.const f32x4 3 3 3 3 v128.const f32x4 4 4 4 4 v128.const f32x4 30 30 30 30 f32x4.relaxed_nmadd f32x4.extract_lane 0)
  (func (export "f32x4_min") (result f32) v128.const f32x4 3 3 3 3 v128.const f32x4 4 4 4 4 f32x4.relaxed_min f32x4.extract_lane 0)
  (func (export "f32x4_max") (result f32) v128.const f32x4 3 3 3 3 v128.const f32x4 4 4 4 4 f32x4.relaxed_max f32x4.extract_lane 0)
  (func (export "trunc_f64x2_s") (result i32) v128.const f64x2 42.75 42.75 i32x4.relaxed_trunc_f64x2_s_zero i32x4.extract_lane 0)
  (func (export "trunc_f64x2_u") (result i32) v128.const f64x2 42.75 42.75 i32x4.relaxed_trunc_f64x2_u_zero i32x4.extract_lane 0)
  (func (export "f64x2_madd") (result f64) v128.const f64x2 3 3 v128.const f64x2 4 4 v128.const f64x2 30 30 f64x2.relaxed_madd f64x2.extract_lane 0)
  (func (export "f64x2_nmadd") (result f64) v128.const f64x2 3 3 v128.const f64x2 4 4 v128.const f64x2 30 30 f64x2.relaxed_nmadd f64x2.extract_lane 0)
  (func (export "f64x2_min") (result f64) v128.const f64x2 3 3 v128.const f64x2 4 4 f64x2.relaxed_min f64x2.extract_lane 0)
  (func (export "f64x2_max") (result f64) v128.const f64x2 3 3 v128.const f64x2 4 4 f64x2.relaxed_max f64x2.extract_lane 0)
  (func (export "i8x16_laneselect") (result i32) v128.const i8x16 42 42 42 42 42 42 42 42 42 42 42 42 42 42 42 42 v128.const i8x16 7 7 7 7 7 7 7 7 7 7 7 7 7 7 7 7 v128.const i8x16 -1 -1 -1 -1 -1 -1 -1 -1 -1 -1 -1 -1 -1 -1 -1 -1 i8x16.relaxed_laneselect i8x16.extract_lane_s 0)
  (func (export "i16x8_laneselect") (result i32) v128.const i16x8 42 42 42 42 42 42 42 42 v128.const i16x8 7 7 7 7 7 7 7 7 v128.const i16x8 -1 -1 -1 -1 -1 -1 -1 -1 i16x8.relaxed_laneselect i16x8.extract_lane_s 0)
  (func (export "i32x4_laneselect") (result i32) v128.const i32x4 42 42 42 42 v128.const i32x4 7 7 7 7 v128.const i32x4 -1 -1 -1 -1 i32x4.relaxed_laneselect i32x4.extract_lane 0)
  (func (export "i64x2_laneselect") (result i64) v128.const i64x2 42 42 v128.const i64x2 7 7 v128.const i64x2 -1 -1 i64x2.relaxed_laneselect i64x2.extract_lane 0)
  (func (export "q15") (result i32) v128.const i16x8 16384 16384 16384 16384 16384 16384 16384 16384 v128.const i16x8 16384 16384 16384 16384 16384 16384 16384 16384 i16x8.relaxed_q15mulr_s i16x8.extract_lane_s 0)
  (func (export "dot") (result i32) v128.const i8x16 2 2 2 2 2 2 2 2 2 2 2 2 2 2 2 2 v128.const i8x16 3 3 3 3 3 3 3 3 3 3 3 3 3 3 3 3 i16x8.relaxed_dot_i8x16_i7x16_s i16x8.extract_lane_s 0)
  (func (export "dot_add") (result i32) v128.const i8x16 2 2 2 2 2 2 2 2 2 2 2 2 2 2 2 2 v128.const i8x16 3 3 3 3 3 3 3 3 3 3 3 3 3 3 3 3 v128.const i32x4 18 18 18 18 i32x4.relaxed_dot_i8x16_i7x16_add_s i32x4.extract_lane 0)
  (func (export "negativeTrunc") (result i32)
    v128.const f32x4 -42.75 -0.9 0 0 i32x4.relaxed_trunc_f32x4_s i32x4.extract_lane 0)
  (func (export "zeroUpper") (result i32)
    v128.const f64x2 42.75 -42.75 i32x4.relaxed_trunc_f64x2_s_zero i32x4.extract_lane 2)
  (func (export "highSwizzle") (result i32)
    v128.const i8x16 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16
    v128.const i8x16 128 129 255 0 0 0 0 0 0 0 0 0 0 0 0 0
    i8x16.relaxed_swizzle i8x16.extract_lane_u 0)
  (func (export "q15Overflow") (result i32)
    v128.const i16x8 -32768 -32768 -32768 -32768 -32768 -32768 -32768 -32768
    v128.const i16x8 -32768 -32768 -32768 -32768 -32768 -32768 -32768 -32768
    i16x8.relaxed_q15mulr_s i16x8.extract_lane_s 0)
  (func (export "dotOverflow") (result i32)
    v128.const i8x16 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128
    v128.const i8x16 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128 -128
    v128.const i32x4 0 0 0 0 i32x4.relaxed_dot_i8x16_i7x16_add_s i32x4.extract_lane 0)
)

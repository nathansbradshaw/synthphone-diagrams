//! Minimal C ABI avoids DOM bindings, text codecs, or JS glue on the audio thread.
use crate::Engine;
#[unsafe(no_mangle)]
pub extern "C" fn engine_new(sr: f32) -> *mut Engine {
    Box::into_raw(Box::new(Engine::new(sr)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_free(e: *mut Engine) {
    drop(unsafe { Box::from_raw(e) });
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_input(e: *mut Engine) -> *mut f32 {
    unsafe { &mut *e }.input_ptr()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_output(e: *mut Engine) -> *const f32 {
    unsafe { &*e }.output_ptr()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_process(e: *mut Engine) {
    unsafe { &mut *e }.process();
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_analyze(e: *mut Engine, view: u8) -> *const f32 {
    let e = unsafe { &mut *e };
    e.analyze(view);
    e.snapshot_ptr()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_configure(e: *mut Engine) {
    let e = unsafe { &mut *e };
    let mut values = [0.; 11];
    values.copy_from_slice(unsafe { std::slice::from_raw_parts(e.input_ptr(), 11) });
    e.configure(&values);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_seek(e: *mut Engine, seconds: f64) {
    unsafe { &mut *e }.seek(seconds);
}
#[unsafe(no_mangle)]
pub extern "C" fn midi_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_midi(e: *mut Engine, ptr: *mut u8, len: usize) -> u32 {
    let data = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
    unsafe { &mut *e }.load_midi(&data).is_ok() as u32
}

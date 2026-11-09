//! Native Android egui application, encrypted rooms and MediaProjection sharing and MediaCodec playback.
#[cfg(target_os = "android")]
mod native;
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    native::run(app);
}

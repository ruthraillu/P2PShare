#[path="profiles.rs"]
mod profiles;
#[path = "audio.rs"]
mod audio;
use eframe::{Frame, egui};
use glow::HasContext;
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
    sys::{jboolean, jint, jstring},
};
use p2pshare_media::{Dimensions, config::VideoFormat};
use p2pshare_room_ui::{Action, Platform, RoomUi};
use p2pshare_session::{Command, Session};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
static SEND_HEVC: AtomicBool=AtomicBool::new(false);
static SEND_BITRATE: AtomicU64=AtomicU64::new(6000);
type AudioEncode=Box<dyn FnMut(&[i16])->Option<Vec<u8>>+Send>;
struct AudioSender { encode: AudioEncode, clock:p2pshare_media::audio::CaptureClock, packets:u64, errors:u64 }
static AUDIO_ENCODER: OnceLock<Mutex<Option<AudioSender>>>=OnceLock::new();
fn sending_codec()->VideoFormat{if SEND_HEVC.load(Ordering::Acquire){VideoFormat::Hevc}else{VideoFormat::H264}}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_ScreenShareService_nativeConfigure(_env:JNIEnv,_class:JClass,hevc:jboolean,bitrate:jint,opus:jint){
    SEND_HEVC.store(hevc!=0,Ordering::Release);SEND_BITRATE.store(bitrate.clamp(100,50000) as u64,Ordering::Release);
    let config=p2pshare_media::audio::AudioConfig{bitrate_kbps:opus.clamp(64,512) as u32,..Default::default()};
    let encoder=p2pshare_media::audio::encoder(config).map(|mut encoder|Box::new(move |pcm:&[i16]|encoder.encode_vec(pcm,1275).ok()) as AudioEncode);
    *AUDIO_ENCODER.get_or_init(||Mutex::new(None)).lock().unwrap()=encoder.ok().map(|encode|AudioSender{encode,clock:Default::default(),packets:0,errors:0});
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_ScreenShareService_nativeAudio(env:JNIEnv,_class:JClass,samples:jni::objects::JShortArray){
    let Some(session)=active_session()else{return};if !SHARING.load(Ordering::Acquire)||!session.needs_live_video(){
        if let Some(sender)=AUDIO_ENCODER.get_or_init(||Mutex::new(None)).lock().unwrap().as_mut(){sender.clock.reset();}return;
    }
    if env.get_array_length(&samples).ok()!=Some(960){return;}
    let mut pcm=[0i16;960];if env.get_short_array_region(&samples,0,&mut pcm).is_err(){return;}
    if let Some(sender)=AUDIO_ENCODER.get_or_init(||Mutex::new(None)).lock().unwrap().as_mut(){
        let now=session.media_micros();let micros=sender.clock.packet(now);
        if let Some(data)=(sender.encode)(&pcm){session.publish_audio(p2pshare_session::AudioPacket{data:data.into(),micros});sender.packets+=1;}
        else{sender.errors+=1;tracing::warn!(errors=sender.errors,"android_audio_encode_failed");}
        if sender.packets%100==0 {tracing::info!(packets=sender.packets,errors=sender.errors,pts_us=micros,callback_us=now,clock_delta_us=now as i128-micros as i128,"android_audio_sender_metrics");}
    }
}
static SHARING: AtomicBool=AtomicBool::new(false);
static CAPTURE_ERROR: OnceLock<Mutex<Option<String>>>=OnceLock::new();
fn active_session()->Option<Arc<Session>>{ACTIVE_SESSION.get().and_then(|m|m.lock().unwrap().as_ref().and_then(std::sync::Weak::upgrade))}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_ScreenShareService_nativeSharing(mut env:JNIEnv,_class:JClass,active:jboolean,width:jint,height:jint,error:JString){
    SHARING.store(active!=0,Ordering::Release);
    if let Some(session)=active_session(){
        if active!=0 {session.set_stream_info(p2pshare_session::StreamInfo{codec:Some(sending_codec()),bitrate_kbps:SEND_BITRATE.load(Ordering::Acquire) as u32});session.command(Command::Publish{width:width as u32,height:height as u32});}
        else{session.command(Command::Unpublish);}
    }
    if let Ok(text)=env.get_string(&error){let text:String=text.into();if !text.is_empty(){*CAPTURE_ERROR.get_or_init(||Mutex::new(None)).lock().unwrap()=Some(text);}}
    if let Some(ctx)=&platform_input().lock().unwrap().repaint{ctx.request_repaint();}
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_ScreenShareService_nativeDemand(_env:JNIEnv,_class:JClass)->jint{
    let Some(s)=active_session()else{return -1};if !s.snapshot().in_room{return -1;}
    if !s.needs_live_video(){return 0;}if s.needs_keyframe(){2}else{1}
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_ScreenShareService_nativeFrame(env:JNIEnv,_class:JClass,bytes:jni::objects::JByteArray,keyframe:jboolean){
    if let Some(s)=active_session(){if s.needs_live_video(){if let Ok(data)=env.convert_byte_array(&bytes){s.publish(p2pshare_session::VideoFrame{codec:sending_codec(),data:data.into(),micros:s.media_micros(),keyframe:keyframe!=0});}}}
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_ScreenShareService_nativePreview(env:JNIEnv,_class:JClass,bytes:jni::objects::JByteArray){
    if let Some(s)=active_session(){if let Ok(data)=env.convert_byte_array(&bytes){s.publish_preview(data);}}
}
#[derive(Default)]
struct PlatformInput {
    back: bool,
    pip: bool,
    pip_closed: bool,
    insets: [f32; 4],
    repaint: Option<egui::Context>,
    available_update: Option<semver::Version>,
}
static PLATFORM_INPUT: OnceLock<Mutex<PlatformInput>> = OnceLock::new();
fn platform_input() -> &'static Mutex<PlatformInput> {
    PLATFORM_INPUT.get_or_init(|| Mutex::new(PlatformInput::default()))
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeVideoReady(
    _env: JNIEnv,
    _class: JClass,
) {
    let repaint = platform_input().lock().unwrap().repaint.clone();
    if let Some(ctx) = repaint {
        ctx.request_repaint();
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeReleaseMetadata(
    mut env: JNIEnv,
    _class: JClass,
    metadata: JString,
) {
    let Ok(metadata) = env.get_string(&metadata) else {
        return;
    };
    let metadata: String = metadata.into();
    let Some(version) = crate::release_update::available(&metadata, &crate::release_update::installed()) else {
        return;
    };
    let repaint = {
        let mut platform = platform_input().lock().unwrap();
        platform.available_update = Some(version);
        platform.repaint.clone()
    };
    if let Some(ctx) = repaint {
        ctx.request_repaint();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeBack(
    _env: JNIEnv,
    _class: JClass,
) {
    platform_input().lock().unwrap().back = true;
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeInsets(
    _env: JNIEnv,
    _class: JClass,
    left: jint,
    top: jint,
    right: jint,
    bottom: jint,
) {
    platform_input().lock().unwrap().insets =
        [left as f32, top as f32, right as f32, bottom as f32];
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativePipMode(_env: JNIEnv, _class: JClass, active: jboolean) {
    let mut state=platform_input().lock().unwrap();state.pip=active!=0;
    if let Some(ctx)=&state.repaint {ctx.request_repaint();}
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativePipClosed(_env: JNIEnv, _class: JClass) {
    platform_input().lock().unwrap().pip_closed=true;
    if let Some(session)=ACTIVE_SESSION.get().and_then(|m|m.lock().unwrap().as_ref().and_then(std::sync::Weak::upgrade)) {
        for peer in session.snapshot().members {session.set_watching(&peer.id,false);}
    }
}
static EDITED_NAME: OnceLock<Mutex<Option<String>>> = OnceLock::new();
fn edited_name() -> &'static Mutex<Option<String>> {
    EDITED_NAME.get_or_init(|| Mutex::new(None))
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeNameEdited(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
) {
    if let Ok(name) = env.get_string(&name) {
        let name: String = name.into();
        *edited_name().lock().unwrap() = Some(name.chars().take(32).collect());
    }
}
static FOREGROUND: AtomicBool = AtomicBool::new(true);
static RESUME_EPOCH: AtomicU64 = AtomicU64::new(0);
static ACTIVE_SESSION: OnceLock<Mutex<Option<std::sync::Weak<Session>>>> = OnceLock::new();
static CONFIG_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
static EDITED_ROOM: OnceLock<Mutex<Option<String>>> = OnceLock::new();
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeForeground(
    _env: JNIEnv,
    _class: JClass,
    active: jboolean,
) {
    let active = active != 0;
    let was_active=FOREGROUND.swap(active, Ordering::AcqRel);
    if active && !was_active {
        RESUME_EPOCH.fetch_add(1, Ordering::AcqRel);
    }
    tracing::info!(active, "android_viewer_foreground_changed");
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeLeaveRoom(
    _env: JNIEnv,
    _class: JClass,
) {
    if let Some(session) = ACTIVE_SESSION.get().and_then(|m| {
        m.lock()
            .unwrap()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
    }) {
        session.command(Command::Leave);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeRoomEdited(
    mut env: JNIEnv,
    _class: JClass,
    room: JString,
) {
    if let Ok(room) = env.get_string(&room) {
        *EDITED_ROOM.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(room.into());
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeSaveConfig(
    mut env: JNIEnv,
    _class: JClass,
    text: JString,
) -> jstring {
    let result = (|| -> anyhow::Result<()> {
        let text: String = env.get_string(&text)?.into();
        p2pshare_session::AppConfig::parse(&text)?;
        std::fs::write(
            CONFIG_PATH
                .get()
                .ok_or_else(|| anyhow::anyhow!("Config path unavailable"))?,
            text,
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => std::ptr::null_mut(),
        Err(error) => env
            .new_string(format!("{error:#}"))
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
    }
}
struct Bridge {
    vm: JavaVM,
    activity: GlobalRef,
}
impl Bridge {
    fn check_for_update(&self) {
        let _ = self.env(|env| {
            env.call_method(self.activity.as_obj(), "checkForUpdate", "()V", &[])?;
            Ok(())
        });
    }

    fn open_update_page(&self, version: &semver::Version) {
        let _ = self.env(|env| {
            let url = env.new_string(crate::release_update::page(version))?;
            env.call_method(
                self.activity.as_obj(),
                "openUpdatePage",
                "(Ljava/lang/String;)V",
                &[JValue::Object(url.as_ref())],
            )?;
            Ok(())
        });
    }

    fn update_pip(&self, active: bool, width: u32, height: u32) {
        let _=self.env(|env| {env.call_method(self.activity.as_obj(),"updatePip","(ZII)V",&[JValue::Bool(active as u8),JValue::Int(width as i32),JValue::Int(height as i32)])?;Ok(())});
    }
    fn enter_pip(&self) {
        let _=self.env(|env| {env.call_method(self.activity.as_obj(),"enterPip","()V",&[])?;Ok(())});
    }

    fn keep_alive(&self, active: bool) {
        let _ = self.env(|env| {
            env.call_method(
                self.activity.as_obj(),
                "keepRoomAlive",
                "(Z)V",
                &[JValue::Bool(active as u8)],
            )?;
            Ok(())
        });
    }
    fn edit_room(&self, room: &str) {
        let _ = self.env(|env| {
            let room = env.new_string(room)?;
            env.call_method(
                self.activity.as_obj(),
                "editRoom",
                "(Ljava/lang/String;)V",
                &[JValue::Object(room.as_ref())],
            )?;
            Ok(())
        });
    }
    fn edit_config(&self) {
        if let Some(path) = CONFIG_PATH.get()
            && let Ok(text) = std::fs::read_to_string(path)
        {
            let _ = self.env(|env| {
                let text = env.new_string(text)?;
                env.call_method(
                    self.activity.as_obj(),
                    "editConfig",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(text.as_ref())],
                )?;
                Ok(())
            });
        }
    }

    fn new(
        app: &winit::platform::android::activity::AndroidApp,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // AndroidApp owns the VM/activity for this application lifetime; retain a JNI global reference.
        unsafe {
            let vm = JavaVM::from_raw(app.vm_as_ptr().cast())?;
            let activity = {
                let env = vm.attach_current_thread()?;
                env.new_global_ref(JObject::from_raw(app.activity_as_ptr().cast()))?
            };
            Ok(Self { vm, activity })
        }
    }
    fn env<T>(
        &self,
        call: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<T>,
    ) -> Result<T, String> {
        let mut env = self.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let result = env.with_local_frame(32, call);
        if result.is_err() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result.map_err(|e| e.to_string())
    }
    fn viewer_mode(&self, fullscreen: bool, handles_back: bool) {
        if let Err(e) = self.env(|env| {
            env.call_method(
                self.activity.as_obj(),
                "setViewerMode",
                "(ZZ)V",
                &[
                    JValue::Bool(fullscreen as u8),
                    JValue::Bool(handles_back as u8),
                ],
            )?;
            Ok(())
        }) {
            log::error!("Viewer mode: {e}");
        }
    }
    fn copy(&self, text: &str) {
        let _ = self.env(|env| {
            let s = env.new_string(text)?;
            env.call_method(
                self.activity.as_obj(),
                "copyInvite",
                "(Ljava/lang/String;)V",
                &[JValue::Object(s.as_ref())],
            )?;
            Ok(())
        });
    }
    fn edit_name(&self, name: &str) {
        let _ = self.env(|env| {
            let name = env.new_string(name)?;
            env.call_method(
                self.activity.as_obj(),
                "editName",
                "(Ljava/lang/String;)V",
                &[JValue::Object(name.as_ref())],
            )?;
            Ok(())
        });
    }
    fn paste(&self) -> String {
        self.env(|env| {
            let s = env
                .call_method(
                    self.activity.as_obj(),
                    "pasteInvite",
                    "()Ljava/lang/String;",
                    &[],
                )?
                .l()?;
            let value = env.get_string(&JString::from(s))?.into();
            Ok(value)
        })
        .unwrap_or_default()
    }
    fn create(&self, peer: &str, texture: u32, size: Dimensions, codec: VideoFormat) -> bool {
        self.env(|env| {
            let p = env.new_string(peer)?;
            let mime = env.new_string(codec.mime())?;
            env.call_method(
                self.activity.as_obj(),
                "createDecoder",
                "(Ljava/lang/String;IIILjava/lang/String;)Z",
                &[
                    JValue::Object(p.as_ref()),
                    JValue::Int(texture as i32),
                    JValue::Int(size.width as i32),
                    JValue::Int(size.height as i32),
                    JValue::Object(mime.as_ref()),
                ],
            )?
            .z()
        })
        .unwrap_or(false)
    }
    fn decoder_diagnostics(&self) -> String {
        self.env(|env| {
            let value = env
                .call_method(
                    self.activity.as_obj(),
                    "decoderDiagnostics",
                    "()Ljava/lang/String;",
                    &[],
                )?
                .l()?;
            Ok(env.get_string((&value).into())?.into())
        })
        .unwrap_or_default()
    }
    fn feed(
        &self,
        peer: &str,
        data: &[u8],
        micros: u64,
        keyframe: bool,
        age_us: u64,
        codec: VideoFormat,
    ) -> bool {
        self.env(|env| {
            let p = env.new_string(peer)?;
            let data = env.byte_array_from_slice(data)?;
            let mime = env.new_string(codec.mime())?;
            env.call_method(
                self.activity.as_obj(),
                "feedDecoder",
                "(Ljava/lang/String;[BJZJLjava/lang/String;)Z",
                &[
                    JValue::Object(p.as_ref()),
                    JValue::Object(data.as_ref()),
                    JValue::Long(micros as i64),
                    JValue::Bool(keyframe as u8),
                    JValue::Long(age_us as i64),
                    JValue::Object(mime.as_ref()),
                ],
            )?
            .z()
        })
        .unwrap_or(false)
    }
    fn update(&self, peer: &str, matrix: &mut [f32; 16]) -> bool {
        self.env(|env| {
            let p = env.new_string(peer)?;
            let array = env.new_float_array(16)?;
            let result = env
                .call_method(
                    self.activity.as_obj(),
                    "updateDecoder",
                    "(Ljava/lang/String;[F)Z",
                    &[JValue::Object(p.as_ref()), JValue::Object(array.as_ref())],
                )?
                .z()?;
            if result {
                env.get_float_array_region(&array, 0, matrix)?;
            }
            Ok(result)
        })
        .unwrap_or(false)
    }
    fn remove(&self, peer: &str) {
        let _ = self.env(|env| {
            let p = env.new_string(peer)?;
            env.call_method(
                self.activity.as_obj(),
                "removeDecoder",
                "(Ljava/lang/String;)V",
                &[JValue::Object(p.as_ref())],
            )?;
            Ok(())
        });
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeDebugLog(
    mut env: JNIEnv,
    _: JClass,
    text: JString,
) {
    if let Ok(text) = env.get_string(&text) {
        p2pshare_media::diagnostics::append(&String::from(text));
    }
}
static DIAGNOSTIC_DEVICE: OnceLock<String> = OnceLock::new();

pub(crate) fn diagnostic_device() -> String {
    DIAGNOSTIC_DEVICE.get().cloned().unwrap_or_else(|| "Android".into())
}

fn log_android_system(bridge: &Bridge) {
    let _ = bridge.env(|env| {
        let model = env.get_static_field("android/os/Build", "MODEL", "Ljava/lang/String;")?.l()?;
        let model: String = env.get_string(&JString::from(model))?.into();
        let _ = DIAGNOSTIC_DEVICE.set(model.clone());
        let release = env.get_static_field("android/os/Build$VERSION", "RELEASE", "Ljava/lang/String;")?.l()?;
        let release: String = env.get_string(&JString::from(release))?.into();
        let api = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
        tracing::warn!(%model, android_release=%release, api, version=env!("CARGO_PKG_VERSION"), "android_diagnostics_system");
        Ok(())
    });
}
pub fn run(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_env_filter("warn,p2pshare_session=info,p2pshare_rtc=info,p2pshare_android=info,p2pshare_media::diagnostics=info")
        .with_writer(|| p2pshare_media::diagnostics::BufferWriter)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let bridge = Arc::new(Bridge::new(&app)?);
        let identity = app
            .internal_data_path()
            .ok_or("Android data directory unavailable")?
            .join("identity.key");
        let config_path = identity.with_file_name("config.toml");
        let config = p2pshare_session::AppConfig::load_or_create(&config_path, "Android")?;
        let data = identity.parent().ok_or("Android data directory unavailable")?.to_owned();
        let archive = match crate::diagnostics::start(&data) {
            Ok((log, guard)) => {
                p2pshare_media::diagnostics::set_archive(Some(Box::new(log)));
                Some(guard)
            }
            Err(error) => {
                log::warn!("Diagnostics archive unavailable: {error:#}");
                None
            }
        };
        p2pshare_media::diagnostics::set_enabled(config.logging_enabled);
        log_android_system(&bridge);
        let _ = CONFIG_PATH.set(config_path);
        let session = Arc::new(Session::start_cloudflare(
            identity,
            config.signaling_url.clone(),
            false,
        )?);
        session.set_playback_config(config.playback)?;
        *ACTIVE_SESSION
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(Arc::downgrade(&session));
        *edited_name().lock().unwrap() = None;
        let mut options = eframe::NativeOptions {
            android_app: Some(app),
            renderer: eframe::Renderer::Glow,
            multisampling: 0,
            depth_buffer: 0,
            stencil_buffer: 0,
            ..Default::default()
        };
        options.glow_options.vsync = true;
        let run_result = eframe::run_native(
            "P2PShare",
            options,
            Box::new(move |cc| {
                RoomUi::configure(&cc.egui_ctx);
                platform_input().lock().unwrap().repaint = Some(cc.egui_ctx.clone());
                bridge.check_for_update();
                let gl = cc.gl.clone().ok_or("OpenGL ES unavailable")?;
                let gpu = Gpu::new(gl)?;
                let mut room = RoomUi::new(true);
                room.set_platform(Platform::Android);
                room.language=config.language.unwrap_or_default();room.language_prompt=config.language.is_none();
                let _=bridge.env(|env|{let code=env.new_string(room.language.code())?;env.call_method(bridge.activity.as_obj(),"setUiLanguage","(Ljava/lang/String;)V",&[JValue::Object(code.as_ref())])?;Ok(())});
                room.logging_enabled = config.logging_enabled;
                let _ = bridge.env(|env| {
                    env.call_method(bridge.activity.as_obj(), "setDebugLogging", "(Z)V",
                        &[JValue::Bool(config.logging_enabled as u8)])?;
                    Ok(())
                });
                room.name = config.username.clone();
                room.room_name = config.room_name.clone();
                room.playback_buffer_ms=config.playback.buffer_ms;
                room.signaling_server=config.signaling_url.clone();
                room.signaling_setup = config.signaling_url.is_empty();
                room.encoder = "MediaCodec AVC sharing / AVC + HEVC viewing".into();
                let receiver = Receiver::new(session.clone(), bridge.clone())?;
                let audio =
                    audio::AudioReceiver::new(session.clone(), bridge.clone(), config.audio)?;
                let profiles=profiles::Store::new(CONFIG_PATH.get().unwrap(),&config,&session);
                Ok(Box::new(AndroidUi {
                    profiles,
                    receiver,
                    _audio: audio,
                    room,
                    session,
                    bridge,
                    gpu,
                    diagnostics_at: std::time::Instant::now(),
                    report_data: data,
                    report_refresh: None,
                    report_upload: None,
                    available_update: None,
                    resume_epoch: RESUME_EPOCH.load(Ordering::Acquire),
                    preview_textures: HashMap::new(),
                }))
            }),
        );
        p2pshare_media::diagnostics::set_archive(None);
        if run_result.is_ok() && let Some(archive) = archive {
            let _ = archive.finish();
        }
        run_result?;
        Ok(())
    })();
    if let Err(error) = result {
        log::error!("P2PShare: {error:#}");
    }
}
// Compressed ingress reaches MediaCodec independently of UI/EGL swap timing.
struct Receiver {
    stopped: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Receiver {
    fn new(session: Arc<Session>, bridge: Arc<Bridge>) -> std::io::Result<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let thread = std::thread::Builder::new()
            .name("p2pshare-video-input".into())
            .spawn(move || {
                let _attachment = match bridge.vm.attach_current_thread() {
                    Ok(attachment) => attachment,
                    Err(error) => {
                        tracing::error!(%error, "decoder_input_jni_attach_failed");
                        return;
                    }
                };
                let mut active = false;
                let mut membership_at = std::time::Instant::now() - Duration::from_secs(1);
                let mut foreground = FOREGROUND.load(Ordering::Acquire);
                while !stop.load(Ordering::Acquire) {
                    if membership_at.elapsed() > Duration::from_millis(250) {
                        membership_at = std::time::Instant::now();
                        let state = session.snapshot();
                        let desired = state.in_room || (state.busy && !state.local_id.is_empty());
                        if active != desired {
                            active = desired;
                            bridge.keep_alive(active);
                            tracing::info!(active, "android_room_presence_changed");
                        }
                    }
                    let visible = FOREGROUND.load(Ordering::Acquire);
                    if !visible {
                        session.discard_playback();
                        foreground = false;
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    if !foreground {
                        foreground = true;
                        for peer in session.snapshot().members.iter().filter(|p| p.sharing) {
                            session.command(Command::RequestKeyframe(peer.id.clone()));
                        }
                    }
                    for packet in session.take_video() {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        if !bridge.feed(
                            &packet.peer,
                            &packet.data,
                            packet.micros,
                            packet.keyframe,
                            packet.received_at.elapsed().as_micros() as u64,
                            packet.codec,
                        ) {
                            session.command(Command::RequestKeyframe(packet.peer));
                        }
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            })?;
        Ok(Self {
            stopped,
            thread: Some(thread),
        })
    }
    fn stop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop();
    }
}
struct AndroidUi {
    profiles:profiles::Store,
    room: RoomUi,
    session: Arc<Session>,
    bridge: Arc<Bridge>,
    receiver: Receiver,
    _audio: audio::AudioReceiver,
    gpu: Gpu,
    diagnostics_at: std::time::Instant,
    report_data: std::path::PathBuf,
    report_refresh: Option<std::time::Instant>,
    report_upload: Option<std::sync::mpsc::Receiver<Result<p2pshare_room_ui::DiagnosticReceipt, String>>>,
    available_update: Option<semver::Version>,
    resume_epoch: u64,
    preview_textures: HashMap<String, egui::TextureHandle>,
}
impl eframe::App for AndroidUi {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        use std::sync::mpsc::TryRecvError;
        let diagnostics = &mut self.room.diagnostic_panel;
        if let Some(upload) = &self.report_upload {
            let result = match upload.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("Diagnostics upload stopped".into())),
            };
            if let Some(result) = result {
                diagnostics.receipt = Some(result);
                diagnostics.sending = false;
                self.report_upload = None;
            }
        }
        if self.report_refresh.is_none_or(|last| last.elapsed().as_secs() >= 2) {
            self.report_refresh = Some(std::time::Instant::now());
            diagnostics.runs = crate::diagnostics::recent(&self.report_data);
            diagnostics.configured = CONFIG_PATH.get().and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| p2pshare_session::AppConfig::parse(&text).ok())
                .is_some_and(|config| crate::diagnostics::upload_url(&config.signaling_url).is_ok());
        }

        {
            let mut platform = platform_input().lock().unwrap();
            self.room
                .set_safe_insets(platform.insets.map(|px| px / ui.ctx().pixels_per_point()));
            self.room.pip_mode=platform.pip;
            if let Some(version) = platform.available_update.take() {
                self.room.android_update_version = Some(version.to_string());
                self.available_update = Some(version);
            }
            if std::mem::take(&mut platform.pip_closed) {
                self.room.watching.clear();self.room.focused=None;self.room.back();
            }
            if std::mem::take(&mut platform.back) {
                self.room.back();
            }
        }
        let epoch = RESUME_EPOCH.load(Ordering::Acquire);
        if epoch != self.resume_epoch {
            self.resume_epoch = epoch;
            self.gpu.reconcile(&[], &self.bridge, frame);
            for peer in &mut self.room.participants {
                if !peer.local {
                    peer.texture = None;
                }
            }
        }
        if let Some(room) = EDITED_ROOM
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .take()
        {
            self.room.room_name = room;
        }
        if let Some(name) = edited_name().lock().unwrap().take() {
            if let Some(path) = CONFIG_PATH.get() {
                let _ = p2pshare_session::AppConfig::save_username(path, &name);
            }
            self.room.name = name.clone();
            self.session.command(Command::Rename(name));
        }
        self.room.capture_running=SHARING.load(Ordering::Acquire);
        if let Some(error)=CAPTURE_ERROR.get_or_init(||Mutex::new(None)).lock().unwrap().take(){self.room.status=format!("Screen sharing failed: {error}");}
        let state = self.session.snapshot();
        self.room.apply_snapshot(&room_state(&state));
        self.profiles.sync(&mut self.room,&self.session,ui.ctx(),&state);
        let desired: Vec<_> = state
            .members
            .iter()
            .filter(|m| m.sharing && m.id != state.local_id && self.session.is_watching(&m.id))
            .map(|m| {
                (
                    m.id.clone(),
                    Dimensions::new(m.width, m.height),
                    self.session.video_codec(&m.id),
                )
            })
            .collect();
        self.gpu.reconcile(&desired, &self.bridge, frame);
        self.preview_textures
            .retain(|id, _| state.members.iter().any(|m| &m.id == id && m.sharing));
        for (peer, bytes) in self.session.take_previews() {
            if let Ok(preview) = p2pshare_media::preview::decode(&bytes) {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [preview.width as usize, preview.height as usize],
                    &preview.rgba,
                );
                let texture = self
                    .preview_textures
                    .entry(peer.clone())
                    .or_insert_with(|| {
                        ui.ctx().load_texture(
                            format!("preview-{peer}"),
                            image.clone(),
                            egui::TextureOptions::LINEAR,
                        )
                    });
                texture.set(image, egui::TextureOptions::LINEAR);
            }
        }
        for (peer, texture) in &self.preview_textures {
            if !self.session.is_watching(peer) {
                let size = texture.size();
                self.room.set_remote_frame(
                    peer,
                    texture.id(),
                    Dimensions::new(size[0] as u32, size[1] as u32),
                );
            }
        }
        for slot in &self.gpu.slots {
            if let Some(peer) = &slot.peer
                && self.gpu.draw_slot(slot, &self.bridge)
            {
                self.room.set_remote_frame(peer, slot.id, slot.size);
            }
        }
        if self.diagnostics_at.elapsed() >= Duration::from_secs(1) {
            self.room.playback_diagnostics = format!(
                "{}\n{}\n{}",
                state.discovery_status,
                self.session.video_diagnostics(),
                self.bridge.decoder_diagnostics()
            );
            if p2pshare_media::diagnostics::enabled() {
                tracing::info!(metrics=%self.session.video_diagnostics(), "receiver_heartbeat");
                p2pshare_media::diagnostics::append(&format!(
                    "{}\n",
                    self.room.playback_diagnostics
                ));
            }
            self.diagnostics_at = std::time::Instant::now();
        }
        self.room.stream_details.clear();
        for member in &state.members {
            if let Some(info) = self.session.stream_info(&member.id) {
                let id = p2pshare_room_ui::peer_ui_id(&member.id);
                let size = self
                    .room
                    .participants
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.size)
                    .unwrap_or(Dimensions::new(member.width, member.height));
                self.room.stream_details.insert(
                    id,
                    format!(
                        "{} × {} · {} · {} kbps",
                        size.width,
                        size.height,
                        info.codec
                            .map(|c| format!("{c:?}").to_uppercase())
                            .unwrap_or_else(|| "Auto".into()),
                        info.bitrate_kbps
                    ),
                );
            }
        }
        let pip_stream=self.room.playback_participant();
        self.bridge.update_pip(pip_stream.is_some(),pip_stream.map(|p|p.size.width).unwrap_or(16),pip_stream.map(|p|p.size.height).unwrap_or(9));
        let actions = self.room.ui(ui);
        for action in actions {
            match action {
                Action::ViewAndroidUpdate => {
                    if let Some(version) = &self.available_update {
                        self.bridge.open_update_page(version);
                    }
                }
                Action::SetLanguage(language)=>{
                    let result=(||->anyhow::Result<()>{let path=CONFIG_PATH.get().ok_or_else(||anyhow::anyhow!("Config unavailable"))?;let mut config=p2pshare_session::AppConfig::parse(&std::fs::read_to_string(path)?)?;config.language=Some(language);let pending=path.with_extension("toml.pending");std::fs::write(&pending,config.document()?)?;std::fs::rename(pending,path)?;Ok(())})();
                    if let Err(error)=result{self.room.language_prompt=true;self.room.status=format!("Could not save language: {error}");}
                    else{let _=self.bridge.env(|env|{let code=env.new_string(language.code())?;env.call_method(self.bridge.activity.as_obj(),"setUiLanguage","(Ljava/lang/String;)V",&[JValue::Object(code.as_ref())])?;Ok(())});}
                },
                Action::ChooseAvatar=>{let _=self.bridge.env(|env|{env.call_method(self.bridge.activity.as_obj(),"chooseAvatar","()V",&[])?;Ok(())});},
                Action::SaveAvatar(bytes)=>{self.room.profiles.error=self.profiles.save(Some(bytes),&self.session).err().map(|e|e.to_string()).unwrap_or_default();},
                Action::RemoveAvatar=>{self.room.profiles.error=self.profiles.save(None,&self.session).err().map(|e|e.to_string()).unwrap_or_default();},
                Action::Watch {
                    participant,
                    enabled,
                } => {
                    for member in self.session.snapshot().members {
                        if p2pshare_room_ui::peer_ui_id(&member.id) == participant {
                            self.session.set_watching(&member.id, enabled);
                        }
                    }
                }
                Action::DebugLogging(enabled) => {
                    p2pshare_media::diagnostics::set_enabled(enabled);
                    if enabled { log_android_system(&self.bridge); }
                    p2pshare_media::diagnostics::append(concat!(
                        "version=",
                        env!("CARGO_PKG_VERSION"),
                        "\n"
                    ));
                    let _ = self.bridge.env(|env| {
                        env.call_method(
                            self.bridge.activity.as_obj(),
                            "setDebugLogging",
                            "(Z)V",
                            &[JValue::Bool(enabled as u8)],
                        )?;
                        Ok(())
                    });
                }
                Action::SendDiagnosticRun(id) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.report_upload = Some(rx);
                    let data = self.report_data.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let result = (|| -> anyhow::Result<_> {
                            let path = CONFIG_PATH.get().ok_or_else(|| anyhow::anyhow!("Configuration unavailable"))?;
                            let config = p2pshare_session::AppConfig::parse(&std::fs::read_to_string(path)?)?;
                            crate::diagnostics::send(&data, &config.signaling_url, &id)
                        })().map_err(|error| format!("{error:#}"));
                        let _ = tx.send(result);
                        ctx.request_repaint();
                    });
                }
                Action::CopyDebugLog => {
                    self.bridge.copy(&p2pshare_media::diagnostics::export());
                    self.room.status = "Debug log copied".into();
                }
                Action::ViewerMode {
                    fullscreen,
                    handles_back,
                } => self.bridge.viewer_mode(fullscreen, handles_back),
                Action::VideoQuality(_) => {}
                Action::AudioVolume {
                    participant,
                    percent,
                } => {
                    for member in self.session.snapshot().members {
                        if p2pshare_room_ui::peer_ui_id(&member.id) == participant {
                            self.session.set_audio_volume(&member.id, percent);
                        }
                    }
                }
                Action::AudioMute { participant, muted } => {
                    for member in &state.members {
                        if p2pshare_room_ui::peer_ui_id(&member.id) == participant {
                            self.session.set_audio_muted(&member.id, muted);
                        }
                    }
                }
                Action::NamedRoom { room, name } => {
                    let result = (|| -> anyhow::Result<()> {
                        let path = CONFIG_PATH.get().ok_or_else(|| anyhow::anyhow!("Config unavailable"))?;
                        let mut config = p2pshare_session::AppConfig::parse(&std::fs::read_to_string(path)?)?;
                        config.room_name = room.trim().to_owned();
                        config.username = name.trim().to_owned();
                        let temporary = path.with_extension("toml.tmp");
                        std::fs::write(&temporary, config.document()?)?;
                        std::fs::rename(temporary, path)?;
                        Ok(())
                    })();
                    if let Err(error) = result {
                        self.room.signaling_setup_error = error.to_string();
                        continue;
                    }
                    self.room.signaling_setup_error.clear();
                    self.bridge.keep_alive(true);
                    self.session.command(Command::NamedRoom { room, name });
                }
                Action::EditRoom(room) => self.bridge.edit_room(&room),
                Action::PictureInPicture => self.bridge.enter_pip(),
                Action::ConfigureSignaling(url) => {
                    let result = (|| -> anyhow::Result<String> {
                        let path = CONFIG_PATH.get().ok_or_else(|| anyhow::anyhow!("Config unavailable"))?;
                        let mut config = p2pshare_session::AppConfig::parse(&std::fs::read_to_string(path)?)?;
                        config.set_signaling_url(&url)?;
                        let temporary = path.with_extension("toml.tmp");
                        std::fs::write(&temporary, config.document()?)?;
                        std::fs::rename(temporary, path)?;
                        Ok(config.signaling_url)
                    })();
                    match result {
                        Ok(url) => {
                            self.session.command(Command::SetSignalingServer(url.clone()));
                            self.room.signaling_server = url;
                            self.room.signaling_setup = false;
                            self.room.signaling_setup_error.clear();
                        }
                        Err(error) => self.room.signaling_setup_error = error.to_string(),
                    }
                }
                Action::SaveMobilePreferences { buffer_ms, server } => {
                    let result=(|| -> anyhow::Result<()> {
                        let path=CONFIG_PATH.get().ok_or_else(||anyhow::anyhow!("Config unavailable"))?;
                        let mut config=p2pshare_session::AppConfig::parse(&std::fs::read_to_string(path)?)?;
                        config.username=self.room.name.clone();config.room_name=self.room.room_name.clone();config.logging_enabled=self.room.logging_enabled;
                        config.playback.buffer_ms=buffer_ms;config.set_signaling_url(&server)?;
                        config.validate()?;let temporary=path.with_extension("toml.tmp");std::fs::write(&temporary,config.document()?)?;std::fs::rename(temporary,path)?;
                        self.session.set_playback_config(config.playback)?;Ok(())
                    })();
                    self.room.status=match result {Ok(())=>"Saved. Restart to apply signaling server changes.".into(),Err(e)=>format!("Settings not saved: {e:#}")};
                }
                Action::EditConfig => self.bridge.edit_config(),
                Action::JoinRoom { invite, name } => {
                    if let Ok(parsed) = p2pshare_session::CloudflareInvite::parse(&invite)
                        && let Some(room) = parsed.room
                    {
                        self.room.room_name = room;
                    }
                    self.bridge.keep_alive(true);
                    self.session.command(Command::Join { invite, name })
                }
                Action::LeaveRoom => {let _=self.bridge.env(|env|{env.call_method(self.bridge.activity.as_obj(),"stopScreenShare","()V",&[])?;Ok(())});self.session.command(Command::Leave);},
                Action::EditName(name) => self.bridge.edit_name(&name),
                Action::Rename(name) => {
                    if let Some(path) = CONFIG_PATH.get() {
                        let _ = p2pshare_session::AppConfig::save_username(path, &name);
                    }
                    self.session.command(Command::Rename(name));
                }
                Action::CopyInvite(text) => self.bridge.copy(&text),
                Action::PasteInvite => self.room.join_input = self.bridge.paste(),
                Action::StartCapture(_) => {let _=self.bridge.env(|env|{env.call_method(self.bridge.activity.as_obj(),"startScreenShare","()V",&[])?;Ok(())});}
                Action::StopCapture => {let _=self.bridge.env(|env|{env.call_method(self.bridge.activity.as_obj(),"stopScreenShare","()V",&[])?;Ok(())});}
                Action::RefreshSources => {}
            }
        }
        if state.in_room && !desired.is_empty() {
            // EGL vsync supplies the pacing. Waiting another 16 ms after a swap
            // can miss the next refresh and reduce a 60 Hz viewer to ~30 FPS.
            ui.ctx().request_repaint();
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }
    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        let _=self.bridge.env(|env|{env.call_method(self.bridge.activity.as_obj(),"stopScreenShare","()V",&[])?;Ok(())});
        self.receiver.stop();
        self.bridge.keep_alive(false);
        platform_input().lock().unwrap().repaint = None;
        self.bridge.viewer_mode(false, false);
        self.session.command(Command::Leave);
        for slot in &self.gpu.slots {
            if let Some(peer) = &slot.peer {
                self.bridge.remove(peer);
            }
        }
        self.gpu.destroy();
    }
}
const OES: u32 = 0x8D65;
struct Slot {
    peer: Option<String>,
    size: Dimensions,
    codec: VideoFormat,
    external: glow::Texture,
    output: glow::Texture,
    fbo: glow::Framebuffer,
    id: egui::TextureId,
}
struct Gpu {
    gl: Arc<glow::Context>,
    program: glow::Program,
    vao: glow::VertexArray,
    matrix: glow::UniformLocation,
    slots: Vec<Slot>,
    failures: HashMap<String, std::time::Instant>,
}
impl Gpu {
    fn new(gl: Arc<glow::Context>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        unsafe {
            let vertex = gl.create_shader(glow::VERTEX_SHADER)?;
            let fragment = gl.create_shader(glow::FRAGMENT_SHADER)?;
            gl.shader_source(vertex,"#version 300 es\nout vec2 uv;void main(){uv=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(uv*2.0-1.0,0,1);}");
            gl.shader_source(fragment,"#version 300 es\n#extension GL_OES_EGL_image_external_essl3 : require\nprecision mediump float;uniform samplerExternalOES image;uniform mat4 transform;in vec2 uv;out vec4 color;void main(){color=texture(image,(transform*vec4(uv.x,1.0-uv.y,0,1)).xy);}");
            for shader in [vertex, fragment] {
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(gl.get_shader_info_log(shader).into());
                }
            }
            let program = gl.create_program()?;
            gl.attach_shader(program, vertex);
            gl.attach_shader(program, fragment);
            gl.link_program(program);
            gl.delete_shader(vertex);
            gl.delete_shader(fragment);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program).into());
            }
            let matrix = gl
                .get_uniform_location(program, "transform")
                .ok_or("Missing transform uniform")?;
            let vao = gl.create_vertex_array()?;
            Ok(Self {
                gl,
                program,
                vao,
                matrix,
                slots: vec![],
                failures: HashMap::new(),
            })
        }
    }
    fn reconcile(
        &mut self,
        desired: &[(String, Dimensions, VideoFormat)],
        bridge: &Bridge,
        frame: &mut Frame,
    ) {
        for slot in &mut self.slots {
            if let Some(peer) = &slot.peer
                && !desired
                    .iter()
                    .any(|(p, size, codec)| p == peer && *size == slot.size && *codec == slot.codec)
            {
                bridge.remove(peer);
                slot.peer = None;
            }
        }
        self.failures
            .retain(|peer, _| desired.iter().any(|(p, _, _)| p == peer));
        for (peer, size, codec) in desired {
            if self
                .failures
                .get(peer)
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
            {
                continue;
            }
            if self.slots.iter().any(|s| s.peer.as_ref() == Some(peer)) {
                continue;
            }
            if size.is_empty() || size.width > 4096 || size.height > 4096 {
                continue;
            }
            if !self.slots.iter().any(|s| s.peer.is_none()) {
                if self.slots.len() >= 6 {
                    continue;
                }
                let result = unsafe {
                    (|| -> Result<Slot, String> {
                        let external = self.gl.create_texture()?;
                        let output = self.gl.create_texture()?;
                        let fbo = self.gl.create_framebuffer()?;
                        self.gl.bind_texture(OES, Some(external));
                        self.gl.tex_parameter_i32(
                            OES,
                            glow::TEXTURE_MIN_FILTER,
                            glow::LINEAR as i32,
                        );
                        self.gl.tex_parameter_i32(
                            OES,
                            glow::TEXTURE_MAG_FILTER,
                            glow::LINEAR as i32,
                        );
                        self.gl.tex_parameter_i32(
                            OES,
                            glow::TEXTURE_WRAP_S,
                            glow::CLAMP_TO_EDGE as i32,
                        );
                        self.gl.tex_parameter_i32(
                            OES,
                            glow::TEXTURE_WRAP_T,
                            glow::CLAMP_TO_EDGE as i32,
                        );
                        let id = frame.register_native_glow_texture(output);
                        Ok(Slot {
                            peer: None,
                            size: *size,
                            codec: *codec,
                            external,
                            output,
                            fbo,
                            id,
                        })
                    })()
                };
                match result {
                    Ok(slot) => self.slots.push(slot),
                    Err(e) => {
                        log::error!("GL texture: {e}");
                        continue;
                    }
                }
            }
            let slot = self.slots.iter_mut().find(|s| s.peer.is_none()).unwrap();
            unsafe {
                self.gl.bind_texture(glow::TEXTURE_2D, Some(slot.output));
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    size.width as i32,
                    size.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
            }
            if bridge.create(peer, slot.external.0.get(), *size, *codec) {
                slot.peer = Some(peer.clone());
                slot.size = *size;
                slot.codec = *codec;
                self.failures.remove(peer);
            } else {
                self.failures
                    .insert(peer.clone(), std::time::Instant::now());
            }
        }
    }
    fn draw_slot(&self, slot: &Slot, bridge: &Bridge) -> bool {
        let mut matrix = [0.0; 16];
        if !bridge.update(slot.peer.as_deref().unwrap(), &mut matrix) {
            return false;
        }
        unsafe {
            let previous = self.gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(slot.fbo));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(slot.output),
                0,
            );
            self.gl
                .viewport(0, 0, slot.size.width as i32, slot.size.height as i32);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.disable(glow::BLEND);
            self.gl.disable(glow::CULL_FACE);
            self.gl.disable(glow::DEPTH_TEST);
            self.gl.use_program(Some(self.program));
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(OES, Some(slot.external));
            self.gl.uniform_1_i32(
                self.gl.get_uniform_location(self.program, "image").as_ref(),
                0,
            );
            self.gl
                .uniform_matrix_4_f32_slice(Some(&self.matrix), false, &matrix);
            self.gl.draw_arrays(glow::TRIANGLES, 0, 3);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
            self.gl.bind_vertex_array(None);
            self.gl.use_program(None);
        }
        true
    }
    fn destroy(&self) {
        unsafe {
            for slot in &self.slots {
                self.gl.delete_texture(slot.external);
                self.gl.delete_framebuffer(slot.fbo);
            }
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_program(self.program);
        }
    }
}

// Translate engine data once at the presentation boundary; UI code has no network dependency.
fn room_state(state: &p2pshare_session::Snapshot) -> p2pshare_room_ui::RoomState {
    p2pshare_room_ui::RoomState {
        local_id: state.local_id.clone(),
        in_room: state.in_room,
        busy: state.busy,
        invite: state.invite.clone(),
        saved_invite: state.saved_invite.clone(),
        status: if state.discovery_status.is_empty() {
            state.status.clone()
        } else {
            format!("{} • {}", state.status, state.discovery_status)
        },
        members: state
            .members
            .iter()
            .map(|m| p2pshare_room_ui::MemberState {
                id: m.id.clone(),
                name: m.name.clone(),
                sharing: m.sharing,
                width: m.width,
                height: m.height,
            })
            .collect(),
    }
}

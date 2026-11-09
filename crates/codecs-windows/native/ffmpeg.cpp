// FFmpeg ABI stays in this small C++ shim, compiled against the pinned runtime headers.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <d3d11.h>
#include <dxgi.h>
#include "libavutil/pixdesc.h"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <limits.h>
#include <initializer_list>
#include <memory>
#include <vector>
#include <atomic>
#include <wrl/client.h>
using Microsoft::WRL::ComPtr;
extern "C" {
#include "libavcodec/avcodec.h"
#include "libavutil/hwcontext.h"
#include "libavutil/hwcontext_d3d11va.h"
#include "libavutil/opt.h"
#include "libswscale/swscale.h"
}
static decltype(&avcodec_version) p_avcodec_version;
static decltype(&avcodec_find_encoder_by_name) p_avcodec_find_encoder_by_name;
static decltype(&avcodec_find_decoder) p_avcodec_find_decoder;
static decltype(&avcodec_alloc_context3) p_avcodec_alloc_context3;
static decltype(&avcodec_free_context) p_avcodec_free_context;
static decltype(&avcodec_open2) p_avcodec_open2;
static decltype(&avcodec_send_frame) p_avcodec_send_frame;
static decltype(&avcodec_receive_packet) p_avcodec_receive_packet;
static decltype(&avcodec_send_packet) p_avcodec_send_packet;
static decltype(&avcodec_receive_frame) p_avcodec_receive_frame;
static decltype(&av_packet_alloc) p_av_packet_alloc;
static decltype(&av_packet_free) p_av_packet_free;
static decltype(&av_packet_unref) p_av_packet_unref;
static decltype(&av_new_packet) p_av_new_packet;
static decltype(&avcodec_get_hw_config) p_avcodec_get_hw_config;
static decltype(&avutil_version) p_avutil_version;
static decltype(&av_log_set_callback) p_av_log_set_callback;
static decltype(&av_pix_fmt_desc_get) p_av_pix_fmt_desc_get;
static decltype(&av_strerror) p_av_strerror;
static decltype(&av_buffer_ref) p_av_buffer_ref;
static decltype(&av_buffer_unref) p_av_buffer_unref;
static decltype(&av_frame_alloc) p_av_frame_alloc;
static decltype(&av_frame_free) p_av_frame_free;
static decltype(&av_frame_unref) p_av_frame_unref;
static decltype(&av_frame_move_ref) p_av_frame_move_ref;
static decltype(&av_frame_get_buffer) p_av_frame_get_buffer;
static decltype(&av_frame_make_writable) p_av_frame_make_writable;
static decltype(&av_hwdevice_ctx_alloc) p_av_hwdevice_ctx_alloc;
static decltype(&av_hwdevice_ctx_init) p_av_hwdevice_ctx_init;
static decltype(&av_hwdevice_ctx_create) p_av_hwdevice_ctx_create;
static decltype(&av_hwframe_ctx_alloc) p_av_hwframe_ctx_alloc;
static decltype(&av_hwframe_ctx_init) p_av_hwframe_ctx_init;
static decltype(&av_hwframe_get_buffer) p_av_hwframe_get_buffer;
static decltype(&av_hwframe_transfer_data) p_av_hwframe_transfer_data;
static decltype(&av_opt_set) p_av_opt_set;
static decltype(&av_opt_set_int) p_av_opt_set_int;
static decltype(&av_dict_set) p_av_dict_set;
static decltype(&av_dict_free) p_av_dict_free;
static decltype(&av_log_set_level) p_av_log_set_level;
static decltype(&sws_getCachedContext) p_sws_getCachedContext;
static decltype(&sws_scale) p_sws_scale;
static decltype(&sws_freeContext) p_sws_freeContext;
static SRWLOCK diagnostics_lock = SRWLOCK_INIT;
static char diagnostics[8192] = {};
static __declspec(thread) char last_diagnostic[1024] = {};
static void codec_log(void *, int level, const char *format, va_list args) {
    if (level > AV_LOG_ERROR) return;
    char line[1024]; vsnprintf(line, sizeof(line), format, args);
    snprintf(last_diagnostic, sizeof(last_diagnostic), "%s", line);
    AcquireSRWLockExclusive(&diagnostics_lock);
    size_t used = strlen(diagnostics), length = strlen(line);
    if (used + length + 1 >= sizeof(diagnostics)) { diagnostics[0] = 0; used = 0; }
    memcpy(diagnostics + used, line, length + 1);
    ReleaseSRWLockExclusive(&diagnostics_lock);
}
extern "C" int ps_ff_diagnostics(char *out, int cap) {
    AcquireSRWLockExclusive(&diagnostics_lock);
    int length = (int)strlen(diagnostics);
    if (length >= cap) length = cap - 1;
    memcpy(out, diagnostics, length); out[length] = 0; diagnostics[0] = 0;
    ReleaseSRWLockExclusive(&diagnostics_lock); return length;
}
extern "C" int ps_ff_init(const wchar_t *util, const wchar_t *codec, const wchar_t *scale, char *error, int cap) {
    HMODULE avutil = LoadLibraryExW(util, NULL, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    HMODULE avcodec = LoadLibraryExW(codec, NULL, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    HMODULE swscale = LoadLibraryExW(scale, NULL, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!avutil || !avcodec || !swscale) { snprintf(error, cap, "Cannot load bundled FFmpeg DLLs (Windows error %lu)", GetLastError()); return -1; }
    p_avcodec_version = reinterpret_cast<decltype(p_avcodec_version)>(GetProcAddress(avcodec, "avcodec_version")); if (!p_avcodec_version) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_version"); return -1; }
    p_avcodec_find_encoder_by_name = reinterpret_cast<decltype(p_avcodec_find_encoder_by_name)>(GetProcAddress(avcodec, "avcodec_find_encoder_by_name")); if (!p_avcodec_find_encoder_by_name) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_find_encoder_by_name"); return -1; }
    p_avcodec_find_decoder = reinterpret_cast<decltype(p_avcodec_find_decoder)>(GetProcAddress(avcodec, "avcodec_find_decoder")); if (!p_avcodec_find_decoder) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_find_decoder"); return -1; }
    p_avcodec_alloc_context3 = reinterpret_cast<decltype(p_avcodec_alloc_context3)>(GetProcAddress(avcodec, "avcodec_alloc_context3")); if (!p_avcodec_alloc_context3) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_alloc_context3"); return -1; }
    p_avcodec_free_context = reinterpret_cast<decltype(p_avcodec_free_context)>(GetProcAddress(avcodec, "avcodec_free_context")); if (!p_avcodec_free_context) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_free_context"); return -1; }
    p_avcodec_open2 = reinterpret_cast<decltype(p_avcodec_open2)>(GetProcAddress(avcodec, "avcodec_open2")); if (!p_avcodec_open2) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_open2"); return -1; }
    p_avcodec_send_frame = reinterpret_cast<decltype(p_avcodec_send_frame)>(GetProcAddress(avcodec, "avcodec_send_frame")); if (!p_avcodec_send_frame) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_send_frame"); return -1; }
    p_avcodec_receive_packet = reinterpret_cast<decltype(p_avcodec_receive_packet)>(GetProcAddress(avcodec, "avcodec_receive_packet")); if (!p_avcodec_receive_packet) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_receive_packet"); return -1; }
    p_avcodec_send_packet = reinterpret_cast<decltype(p_avcodec_send_packet)>(GetProcAddress(avcodec, "avcodec_send_packet")); if (!p_avcodec_send_packet) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_send_packet"); return -1; }
    p_avcodec_receive_frame = reinterpret_cast<decltype(p_avcodec_receive_frame)>(GetProcAddress(avcodec, "avcodec_receive_frame")); if (!p_avcodec_receive_frame) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_receive_frame"); return -1; }
    p_av_packet_alloc = reinterpret_cast<decltype(p_av_packet_alloc)>(GetProcAddress(avcodec, "av_packet_alloc")); if (!p_av_packet_alloc) { snprintf(error, cap, "Missing FFmpeg symbol av_packet_alloc"); return -1; }
    p_av_packet_free = reinterpret_cast<decltype(p_av_packet_free)>(GetProcAddress(avcodec, "av_packet_free")); if (!p_av_packet_free) { snprintf(error, cap, "Missing FFmpeg symbol av_packet_free"); return -1; }
    p_av_packet_unref = reinterpret_cast<decltype(p_av_packet_unref)>(GetProcAddress(avcodec, "av_packet_unref")); if (!p_av_packet_unref) { snprintf(error, cap, "Missing FFmpeg symbol av_packet_unref"); return -1; }
    p_av_new_packet = reinterpret_cast<decltype(p_av_new_packet)>(GetProcAddress(avcodec, "av_new_packet")); if (!p_av_new_packet) { snprintf(error, cap, "Missing FFmpeg symbol av_new_packet"); return -1; }
    p_avcodec_get_hw_config = reinterpret_cast<decltype(p_avcodec_get_hw_config)>(GetProcAddress(avcodec, "avcodec_get_hw_config")); if (!p_avcodec_get_hw_config) { snprintf(error, cap, "Missing FFmpeg symbol avcodec_get_hw_config"); return -1; }
    p_avutil_version = reinterpret_cast<decltype(p_avutil_version)>(GetProcAddress(avutil, "avutil_version")); if (!p_avutil_version) { snprintf(error, cap, "Missing FFmpeg symbol avutil_version"); return -1; }
    p_av_strerror = reinterpret_cast<decltype(p_av_strerror)>(GetProcAddress(avutil, "av_strerror")); if (!p_av_strerror) { snprintf(error, cap, "Missing FFmpeg symbol av_strerror"); return -1; }
    p_av_buffer_ref = reinterpret_cast<decltype(p_av_buffer_ref)>(GetProcAddress(avutil, "av_buffer_ref")); if (!p_av_buffer_ref) { snprintf(error, cap, "Missing FFmpeg symbol av_buffer_ref"); return -1; }
    p_av_buffer_unref = reinterpret_cast<decltype(p_av_buffer_unref)>(GetProcAddress(avutil, "av_buffer_unref")); if (!p_av_buffer_unref) { snprintf(error, cap, "Missing FFmpeg symbol av_buffer_unref"); return -1; }
    p_av_frame_alloc = reinterpret_cast<decltype(p_av_frame_alloc)>(GetProcAddress(avutil, "av_frame_alloc")); if (!p_av_frame_alloc) { snprintf(error, cap, "Missing FFmpeg symbol av_frame_alloc"); return -1; }
    p_av_frame_free = reinterpret_cast<decltype(p_av_frame_free)>(GetProcAddress(avutil, "av_frame_free")); if (!p_av_frame_free) { snprintf(error, cap, "Missing FFmpeg symbol av_frame_free"); return -1; }
    p_av_frame_unref = reinterpret_cast<decltype(p_av_frame_unref)>(GetProcAddress(avutil, "av_frame_unref")); if (!p_av_frame_unref) { snprintf(error, cap, "Missing FFmpeg symbol av_frame_unref"); return -1; }
    p_av_frame_move_ref = reinterpret_cast<decltype(p_av_frame_move_ref)>(GetProcAddress(avutil, "av_frame_move_ref")); if (!p_av_frame_move_ref) return -1;
    p_av_frame_get_buffer = reinterpret_cast<decltype(p_av_frame_get_buffer)>(GetProcAddress(avutil, "av_frame_get_buffer")); if (!p_av_frame_get_buffer) { snprintf(error, cap, "Missing FFmpeg symbol av_frame_get_buffer"); return -1; }
    p_av_frame_make_writable = reinterpret_cast<decltype(p_av_frame_make_writable)>(GetProcAddress(avutil, "av_frame_make_writable")); if (!p_av_frame_make_writable) { snprintf(error, cap, "Missing FFmpeg symbol av_frame_make_writable"); return -1; }
    p_av_hwdevice_ctx_alloc = reinterpret_cast<decltype(p_av_hwdevice_ctx_alloc)>(GetProcAddress(avutil, "av_hwdevice_ctx_alloc")); if (!p_av_hwdevice_ctx_alloc) { snprintf(error, cap, "Missing FFmpeg symbol av_hwdevice_ctx_alloc"); return -1; }
    p_av_hwdevice_ctx_init = reinterpret_cast<decltype(p_av_hwdevice_ctx_init)>(GetProcAddress(avutil, "av_hwdevice_ctx_init")); if (!p_av_hwdevice_ctx_init) { snprintf(error, cap, "Missing FFmpeg symbol av_hwdevice_ctx_init"); return -1; }
    p_av_hwdevice_ctx_create = reinterpret_cast<decltype(p_av_hwdevice_ctx_create)>(GetProcAddress(avutil, "av_hwdevice_ctx_create")); if (!p_av_hwdevice_ctx_create) { snprintf(error, cap, "Missing FFmpeg symbol av_hwdevice_ctx_create"); return -1; }
    p_av_hwframe_ctx_alloc = reinterpret_cast<decltype(p_av_hwframe_ctx_alloc)>(GetProcAddress(avutil, "av_hwframe_ctx_alloc")); if (!p_av_hwframe_ctx_alloc) { snprintf(error, cap, "Missing FFmpeg symbol av_hwframe_ctx_alloc"); return -1; }
    p_av_hwframe_ctx_init = reinterpret_cast<decltype(p_av_hwframe_ctx_init)>(GetProcAddress(avutil, "av_hwframe_ctx_init")); if (!p_av_hwframe_ctx_init) { snprintf(error, cap, "Missing FFmpeg symbol av_hwframe_ctx_init"); return -1; }
    p_av_hwframe_get_buffer = reinterpret_cast<decltype(p_av_hwframe_get_buffer)>(GetProcAddress(avutil, "av_hwframe_get_buffer")); if (!p_av_hwframe_get_buffer) { snprintf(error, cap, "Missing FFmpeg symbol av_hwframe_get_buffer"); return -1; }
    p_av_hwframe_transfer_data = reinterpret_cast<decltype(p_av_hwframe_transfer_data)>(GetProcAddress(avutil, "av_hwframe_transfer_data")); if (!p_av_hwframe_transfer_data) { snprintf(error, cap, "Missing FFmpeg symbol av_hwframe_transfer_data"); return -1; }
    p_av_opt_set = reinterpret_cast<decltype(p_av_opt_set)>(GetProcAddress(avutil, "av_opt_set")); if (!p_av_opt_set) { snprintf(error, cap, "Missing FFmpeg symbol av_opt_set"); return -1; }
    p_av_opt_set_int = reinterpret_cast<decltype(p_av_opt_set_int)>(GetProcAddress(avutil, "av_opt_set_int")); if (!p_av_opt_set_int) { snprintf(error, cap, "Missing FFmpeg symbol av_opt_set_int"); return -1; }
    p_av_dict_set = reinterpret_cast<decltype(p_av_dict_set)>(GetProcAddress(avutil, "av_dict_set")); if (!p_av_dict_set) { snprintf(error, cap, "Missing FFmpeg symbol av_dict_set"); return -1; }
    p_av_dict_free = reinterpret_cast<decltype(p_av_dict_free)>(GetProcAddress(avutil, "av_dict_free")); if (!p_av_dict_free) { snprintf(error, cap, "Missing FFmpeg symbol av_dict_free"); return -1; }
    p_av_log_set_level = reinterpret_cast<decltype(p_av_log_set_level)>(GetProcAddress(avutil, "av_log_set_level")); if (!p_av_log_set_level) { snprintf(error, cap, "Missing FFmpeg symbol av_log_set_level"); return -1; }
    p_sws_getCachedContext = reinterpret_cast<decltype(p_sws_getCachedContext)>(GetProcAddress(swscale, "sws_getCachedContext")); if (!p_sws_getCachedContext) { snprintf(error, cap, "Missing FFmpeg symbol sws_getCachedContext"); return -1; }
    p_sws_scale = reinterpret_cast<decltype(p_sws_scale)>(GetProcAddress(swscale, "sws_scale")); if (!p_sws_scale) { snprintf(error, cap, "Missing FFmpeg symbol sws_scale"); return -1; }
    p_sws_freeContext = reinterpret_cast<decltype(p_sws_freeContext)>(GetProcAddress(swscale, "sws_freeContext")); if (!p_sws_freeContext) { snprintf(error, cap, "Missing FFmpeg symbol sws_freeContext"); return -1; }
    if ((p_avcodec_version() >> 16) != LIBAVCODEC_VERSION_MAJOR || (p_avutil_version() >> 16) != LIBAVUTIL_VERSION_MAJOR) {
        snprintf(error, cap, "Bundled FFmpeg ABI version mismatch"); return -1;
    }
    p_av_pix_fmt_desc_get = reinterpret_cast<decltype(p_av_pix_fmt_desc_get)>(GetProcAddress(avutil, "av_pix_fmt_desc_get"));
    if (!p_av_pix_fmt_desc_get) return -1;
    p_av_log_set_callback = reinterpret_cast<decltype(p_av_log_set_callback)>(GetProcAddress(avutil, "av_log_set_callback"));
    if (!p_av_log_set_callback) return -1;
    p_av_log_set_callback(codec_log);
    p_av_log_set_level(AV_LOG_ERROR);
    return 0;
}
#define avcodec_version p_avcodec_version
#define avcodec_find_encoder_by_name p_avcodec_find_encoder_by_name
#define avcodec_find_decoder p_avcodec_find_decoder
#define avcodec_alloc_context3 p_avcodec_alloc_context3
#define avcodec_free_context p_avcodec_free_context
#define avcodec_open2 p_avcodec_open2
#define avcodec_send_frame p_avcodec_send_frame
#define avcodec_receive_packet p_avcodec_receive_packet
#define avcodec_send_packet p_avcodec_send_packet
#define avcodec_receive_frame p_avcodec_receive_frame
#define av_packet_alloc p_av_packet_alloc
#define av_packet_free p_av_packet_free
#define av_packet_unref p_av_packet_unref
#define av_new_packet p_av_new_packet
#define avcodec_get_hw_config p_avcodec_get_hw_config
#define avutil_version p_avutil_version
#define av_pix_fmt_desc_get p_av_pix_fmt_desc_get
#define av_strerror p_av_strerror
#define av_buffer_ref p_av_buffer_ref
#define av_buffer_unref p_av_buffer_unref
#define av_frame_alloc p_av_frame_alloc
#define av_frame_free p_av_frame_free
#define av_frame_unref p_av_frame_unref
#define av_frame_move_ref p_av_frame_move_ref
#define av_frame_get_buffer p_av_frame_get_buffer
#define av_frame_make_writable p_av_frame_make_writable
#define av_hwdevice_ctx_alloc p_av_hwdevice_ctx_alloc
#define av_hwdevice_ctx_init p_av_hwdevice_ctx_init
#define av_hwdevice_ctx_create p_av_hwdevice_ctx_create
#define av_hwframe_ctx_alloc p_av_hwframe_ctx_alloc
#define av_hwframe_ctx_init p_av_hwframe_ctx_init
#define av_hwframe_get_buffer p_av_hwframe_get_buffer
#define av_hwframe_transfer_data p_av_hwframe_transfer_data
#define av_opt_set p_av_opt_set
#define av_opt_set_int p_av_opt_set_int
#define av_dict_set p_av_dict_set
#define av_dict_free p_av_dict_free
#define av_log_set_level p_av_log_set_level
#define sws_getCachedContext p_sws_getCachedContext
#define sws_scale p_sws_scale
#define sws_freeContext p_sws_freeContext

static void message(char *error, int cap, const char *where, int result) {
    char text[160]; av_strerror(result, text, sizeof(text)); snprintf(error, cap, "%s: %s (%d); %s", where, text, result, last_diagnostic);
}
// A lease keeps a display surface unavailable until the UI has replaced it.
// Shared ownership also makes decoder teardown/resize safe while a frame is visible.
struct DisplaySlot {
    ComPtr<ID3D11Texture2D> texture;
    ComPtr<ID3D11VideoProcessorOutputView> view;
    std::atomic<bool> busy{false};
};
struct DisplayLease { std::shared_ptr<DisplaySlot> slot; };
struct DisplayConverter {
    ComPtr<ID3D11VideoProcessorEnumerator> enumerator;
    ComPtr<ID3D11VideoProcessor> processor;
    std::vector<std::shared_ptr<DisplaySlot>> slots;
    int width = 0, height = 0;
};
extern "C" void ps_ff_display_release(DisplayLease *lease) {
    if (lease) { lease->slot->busy.store(false); delete lease; }
}
extern "C" ID3D11Texture2D *ps_ff_display_texture(DisplayLease *lease) {
    return lease ? lease->slot->texture.Get() : NULL;
}
struct Codec {
    AVCodecContext *ctx;
    AVFrame *frame;
    AVFrame *download;
    AVPacket *packet;
    AVBufferRef *device;
    AVBufferRef *frames;
    SwsContext *scale;
    int hardware;
    int gpu_input;
    DisplayConverter display;
};
extern "C" void ps_ff_close(Codec *s) {
    if (!s) return;
    avcodec_free_context(&s->ctx); av_frame_free(&s->frame); av_frame_free(&s->download);
    av_packet_free(&s->packet); av_buffer_unref(&s->frames); av_buffer_unref(&s->device);
    sws_freeContext(s->scale); delete s;
}
static int wrap_device(Codec *s, ID3D11Device *device) {
    s->device = av_hwdevice_ctx_alloc(AV_HWDEVICE_TYPE_D3D11VA);
    if (!s->device) return AVERROR(ENOMEM);
    auto hw = (AVHWDeviceContext *)s->device->data;
    auto d3d = (AVD3D11VADeviceContext *)hw->hwctx;
    device->AddRef(); d3d->device = device;
    return av_hwdevice_ctx_init(s->device);
}
extern "C" ID3D11Device *ps_ff_device(int index);
extern "C" long long ps_ff_device_luid(ID3D11Device *device);
extern "C" Codec *ps_ff_encoder(const char *name, int width, int height, int fps, int bitrate, int threads,
                                  ID3D11Device *device, int adapter, char *error, int cap) {
    const AVCodec *codec = avcodec_find_encoder_by_name(name);
    if (!codec) { snprintf(error, cap, "Encoder not bundled: %s", name); return NULL; }
    Codec *s = new Codec{};
    s->ctx = avcodec_alloc_context3(codec); s->frame = av_frame_alloc(); s->packet = av_packet_alloc();
    if (!s->ctx || !s->frame || !s->packet) { ps_ff_close(s); return NULL; }
    AVCodecContext *c = s->ctx;
    c->width = width; c->height = height; c->time_base = AVRational{1, 1000000}; c->framerate = AVRational{fps, 1};
    c->bit_rate = bitrate; c->rc_max_rate = bitrate; c->rc_buffer_size = bitrate / fps * 2;
    // Preserve existing low-latency levels through 1080p. Larger presets need
    // the encoder to choose a level matching both dimensions and frame rate.
    if (codec->id == AV_CODEC_ID_H264 && (int64_t)width * height <= 1920LL * 1080)
        c->level = fps <= 60 ? 42 : 51;
    // Live viewers request IDRs for join/loss recovery. Avoid two-second quality
    // resets. QSV's GOP field is 16-bit; use its safe long-GOP range explicitly.
    c->gop_size = strstr(name, "qsv") ? INT16_MAX : INT_MAX;
    c->max_b_frames = 0; c->thread_count = threads; c->flags |= AV_CODEC_FLAG_LOW_DELAY;
    c->color_range = AVCOL_RANGE_MPEG; c->colorspace = AVCOL_SPC_SMPTE170M;
    c->color_primaries = AVCOL_PRI_BT709; c->color_trc = AVCOL_TRC_BT709;
    bool software = strstr(name, "libx") != NULL;
    c->pix_fmt = AV_PIX_FMT_YUV420P;
    if (!software) { c->pix_fmt = AV_PIX_FMT_NV12; s->hardware = 1; }
    int result = 0;
    if (!software && !device) {
        if (strstr(name, "qsv")) {
            AVDictionary *options = NULL; char child[20]; snprintf(child, sizeof(child), "%d", adapter);
            av_dict_set(&options, "child_device", child, 0); av_dict_set(&options, "child_device_type", "d3d11va", 0);
            result = av_hwdevice_ctx_create(&s->device, AV_HWDEVICE_TYPE_QSV, "hw_any", options, 0); av_dict_free(&options);
        } else {
            ID3D11Device *selected = ps_ff_device(adapter);
            if (!selected) result = AVERROR(ENODEV);
            else { result = wrap_device(s, selected); selected->Release(); }
        }
        if (result < 0) goto fail;
        c->hw_device_ctx = av_buffer_ref(s->device);
    }
    // NVENC and AMF directly consume D3D11 NV12 surfaces. QSV uses NV12 upload initially.
    if (device && !software && !strstr(name, "qsv")) {
        if ((result = wrap_device(s, device)) < 0) goto fail;
        s->frames = av_hwframe_ctx_alloc(s->device);
        if (!s->frames) { result = AVERROR(ENOMEM); goto fail; }
        auto f = (AVHWFramesContext *)s->frames->data;
        f->format = AV_PIX_FMT_D3D11; f->sw_format = AV_PIX_FMT_NV12;
        f->width = width; f->height = height; f->initial_pool_size = 0;
        auto d3d = (AVD3D11VAFramesContext *)f->hwctx;
        d3d->BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
        if ((result = av_hwframe_ctx_init(s->frames)) < 0) goto fail;
        c->hw_frames_ctx = av_buffer_ref(s->frames); c->pix_fmt = AV_PIX_FMT_D3D11; s->gpu_input = 1;
    }
    if (strstr(name, "nvenc")) {
        av_opt_set(c->priv_data, "preset", "p1", 0); av_opt_set(c->priv_data, "tune", "ull", 0);
        av_opt_set(c->priv_data, "rc", "cbr", 0); av_opt_set_int(c->priv_data, "rc-lookahead", 0, 0);
        av_opt_set_int(c->priv_data, "zerolatency", 1, 0); av_opt_set_int(c->priv_data, "delay", 0, 0);
        av_opt_set_int(c->priv_data, "forced-idr", 1, 0);
        av_opt_set_int(c->priv_data, "no-scenecut", 1, 0);
        if (!device) av_opt_set_int(c->priv_data, "gpu", adapter, 0);
        if (codec->id == AV_CODEC_ID_H264) av_opt_set(c->priv_data, "profile", "baseline", 0);
    } else if (strstr(name, "amf")) {
        av_opt_set(c->priv_data, "usage", "ultralowlatency", 0); av_opt_set(c->priv_data, "quality", "speed", 0);
        av_opt_set(c->priv_data, "rc", "cbr", 0); av_opt_set_int(c->priv_data, "preanalysis", 0, 0);
        av_opt_set_int(c->priv_data, "async_depth", 1, 0); av_opt_set_int(c->priv_data, "header_insertion_mode", 2, 0);
        // AMF defaults requested I frames to non-IDR frames. New/recovering
        // viewers need a true IDR plus SPS/PPS (VPS/SPS/PPS for HEVC).
        // FFmpeg inserts those headers when forced_idr is enabled.
        result = av_opt_set_int(c->priv_data, "forced_idr", 1, 0);
        if (result < 0) goto fail;
        if (codec->id == AV_CODEC_ID_H264) {
            result = av_opt_set(c->priv_data, "profile", "constrained_baseline", 0);
            if (result < 0) goto fail;
        }
    } else if (strstr(name, "qsv")) {
        av_opt_set(c->priv_data, "preset", "veryfast", 0); av_opt_set_int(c->priv_data, "async_depth", 1, 0);
        av_opt_set_int(c->priv_data, "look_ahead", 0, 0); av_opt_set_int(c->priv_data, "forced_idr", 1, 0);
    } else {
        av_opt_set(c->priv_data, "preset", "veryfast", 0); av_opt_set(c->priv_data, "tune", "zerolatency", 0);
        // A recovery/join frame must be independently decodable, not merely I-coded.
        av_opt_set_int(c->priv_data, "forced-idr", 1, 0);
        if (codec->id == AV_CODEC_ID_H264) {
            av_opt_set(c->priv_data, "profile", "baseline", 0);
            av_opt_set(c->priv_data, "x264-params", "annexb=1:repeat-headers=1:scenecut=0:rc-lookahead=0", 0);
        } else {
            char params[200]; snprintf(params, sizeof(params), "annexb=1:repeat-headers=1:keyint=-1:scenecut=0:rc-lookahead=0:bframes=0:pools=none:frame-threads=%d:wpp=0:log-level=error", threads);
            av_opt_set(c->priv_data, "x265-params", params, 0);
        }
    }
    result = avcodec_open2(c, codec, NULL);
    if (result < 0) goto fail;
    if (!s->gpu_input) {
        s->frame->format = c->pix_fmt; s->frame->width = width; s->frame->height = height;
        if ((result = av_frame_get_buffer(s->frame, 32)) < 0) goto fail;
    }
    return s;
fail:
    message(error, cap, name, result); ps_ff_close(s); return NULL;
}
extern "C" int ps_ff_encode(Codec *s, const unsigned char *bgra, int stride, ID3D11Texture2D *texture,
                              int64_t micros, int keyframe, char *error, int cap) {
    int result;
    if (s->gpu_input) {
        if (!texture) return AVERROR(EINVAL);
        av_frame_unref(s->frame);
        if ((result = av_hwframe_get_buffer(s->frames, s->frame, 0)) < 0) { message(error, cap, "GPU frame allocation", result); return result; }
        auto hw = (AVHWDeviceContext *)s->device->data;
        auto d3d = (AVD3D11VADeviceContext *)hw->hwctx;
        d3d->lock(d3d->lock_ctx);
        d3d->device_context->CopySubresourceRegion((ID3D11Texture2D *)s->frame->data[0], (UINT)(intptr_t)s->frame->data[1], 0, 0, 0, texture, 0, NULL);
        d3d->device_context->Flush(); d3d->unlock(d3d->lock_ctx);
    } else {
        if (!bgra) return AVERROR(EINVAL);
        if ((result = av_frame_make_writable(s->frame)) < 0) return result;
        s->scale = sws_getCachedContext(s->scale, s->ctx->width, s->ctx->height, AV_PIX_FMT_BGRA,
                 s->ctx->width, s->ctx->height, s->ctx->pix_fmt, SWS_FAST_BILINEAR, NULL, NULL, NULL);
        if (!s->scale) return AVERROR(ENOMEM);
        const uint8_t *input[4] = {bgra, NULL, NULL, NULL}; int lines[4] = {stride, 0, 0, 0};
        if ((result = sws_scale(s->scale, input, lines, 0, s->ctx->height, s->frame->data, s->frame->linesize)) < 0) return result;
    }
    s->frame->pts = micros; s->frame->pict_type = keyframe ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;
    result = avcodec_send_frame(s->ctx, s->frame);
    if (result < 0) message(error, cap, "Encode input", result);
    return result;
}
extern "C" int ps_ff_packet(Codec *s, const unsigned char **data, int *size, int64_t *micros, int *keyframe, char *error, int cap) {
    av_packet_unref(s->packet); int result = avcodec_receive_packet(s->ctx, s->packet);
    if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) return 0;
    if (result < 0) { message(error, cap, "Encode output", result); return result; }
    *data = s->packet->data; *size = s->packet->size; *micros = s->packet->pts; *keyframe = !!(s->packet->flags & AV_PKT_FLAG_KEY);
    return 1;
}
static AVPixelFormat select_format(AVCodecContext *ctx, const AVPixelFormat *formats) {
    Codec *s = (Codec *)ctx->opaque;
    if (s->device) { for (auto p = formats; *p != AV_PIX_FMT_NONE; ++p) if (*p == AV_PIX_FMT_D3D11) { s->hardware = 1; return *p; } }
    s->hardware = 0;
    for (auto p = formats; *p != AV_PIX_FMT_NONE; ++p) if (const auto desc = av_pix_fmt_desc_get(*p); desc && !(desc->flags & AV_PIX_FMT_FLAG_HWACCEL)) return *p;
    return AV_PIX_FMT_NONE;
}
extern "C" Codec *ps_ff_decoder(int hevc, int threads, int hardware, int adapter, ID3D11Device *device, char *error, int cap) {
    const AVCodec *codec = avcodec_find_decoder(hevc ? AV_CODEC_ID_HEVC : AV_CODEC_ID_H264);
    if (!codec) return NULL;
    Codec *s = new Codec{};
    s->ctx = avcodec_alloc_context3(codec); s->frame = av_frame_alloc(); s->download = av_frame_alloc(); s->packet = av_packet_alloc();
    if (!s->ctx || !s->frame || !s->download || !s->packet) { ps_ff_close(s); return NULL; }
    int result;
    if (hardware) {
        char gpu[20]; snprintf(gpu, sizeof(gpu), "%d", adapter);
        result = device ? wrap_device(s, device) : av_hwdevice_ctx_create(&s->device, AV_HWDEVICE_TYPE_D3D11VA, gpu, NULL, 0);
        if (result < 0) goto fail;
        s->ctx->hw_device_ctx = av_buffer_ref(s->device);
    }
    s->ctx->opaque = s; s->ctx->get_format = select_format;
    s->ctx->thread_count = hardware ? 1 : threads; s->ctx->thread_type = FF_THREAD_SLICE;
    s->ctx->flags |= AV_CODEC_FLAG_LOW_DELAY; s->ctx->max_pixels = 4096LL * 2160;
    if ((result = avcodec_open2(s->ctx, codec, NULL)) < 0) goto fail;
    return s;
fail:
    message(error, cap, "Decoder initialization", result); ps_ff_close(s); return NULL;
}
// Report the D3D11 device actually retained by FFmpeg after codec opening.
// QSV uses its explicitly selected child adapter and has no public D3D field here.
extern "C" long long ps_ff_codec_device_luid(Codec *s) {
    if (!s || !s->device) return 0;
    auto hw = (AVHWDeviceContext *)s->device->data;
    if (hw->type != AV_HWDEVICE_TYPE_D3D11VA) return 0;
    auto d3d = (AVD3D11VADeviceContext *)hw->hwctx;
    return d3d->device ? ps_ff_device_luid(d3d->device) : 0;
}
extern "C" void ps_ff_cpu_info(char *out, int cap) {
    char name[256] = "Unknown CPU", vendor[96] = "unknown"; HKEY key;
    if (RegOpenKeyExA(HKEY_LOCAL_MACHINE, "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0", 0, KEY_READ, &key) == ERROR_SUCCESS) {
        DWORD size = sizeof(name)-1; RegQueryValueExA(key, "ProcessorNameString", NULL, NULL, (LPBYTE)name, &size);
        size = sizeof(vendor)-1; RegQueryValueExA(key, "VendorIdentifier", NULL, NULL, (LPBYTE)vendor, &size); RegCloseKey(key);
    }
    snprintf(out, cap, "%s; vendor=%s; logical_processors=%lu", name, vendor, GetActiveProcessorCount(ALL_PROCESSOR_GROUPS));
}
extern "C" int ps_ff_decode(Codec *s, const unsigned char *data, int size, char *error, int cap) {
    av_packet_unref(s->packet); int result = av_new_packet(s->packet, size);
    if (result < 0) return result;
    memcpy(s->packet->data, data, size);
    result = avcodec_send_packet(s->ctx, s->packet);
    if (result < 0) message(error, cap, "Decode input", result);
    return result;
}
// Uses the adapter's standard D3D11 video processor, without CPU pixel access.
// All calls use the decoder device's protected immediate context; no cross-GPU copies.
static HRESULT display_frame(Codec *s, AVFrame *f, DisplayLease **lease) {
    auto hw = (AVHWDeviceContext *)s->device->data;
    auto d3d = (AVD3D11VADeviceContext *)hw->hwctx;
    auto &out = s->display;
    HRESULT hr;
    if (out.width != f->width || out.height != f->height) {
        out = DisplayConverter{};
        D3D11_VIDEO_PROCESSOR_CONTENT_DESC content{};
        content.InputFrameFormat = D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE;
        content.InputWidth = content.OutputWidth = f->width;
        content.InputHeight = content.OutputHeight = f->height;
        content.InputFrameRate = content.OutputFrameRate = DXGI_RATIONAL{60, 1};
        content.Usage = D3D11_VIDEO_USAGE_PLAYBACK_NORMAL;
        if (FAILED(hr = d3d->video_device->CreateVideoProcessorEnumerator(&content, &out.enumerator))) return hr;
        UINT support = 0;
        if (FAILED(hr = out.enumerator->CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM, &support))) return hr;
        if (!(support & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT)) return E_NOTIMPL;
        if (FAILED(hr = d3d->video_device->CreateVideoProcessor(out.enumerator.Get(), 0, &out.processor))) return hr;
        D3D11_TEXTURE2D_DESC desc{};
        desc.Width = f->width; desc.Height = f->height;
        desc.MipLevels = desc.ArraySize = 1; desc.SampleDesc.Count = 1;
        desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM; desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC view{};
        view.ViewDimension = D3D11_VPOV_DIMENSION_TEXTURE2D;
        for (int i = 0; i < 4; ++i) {
            auto slot = std::make_shared<DisplaySlot>();
            if (FAILED(hr = d3d->device->CreateTexture2D(&desc, NULL, &slot->texture))) return hr;
            if (FAILED(hr = d3d->video_device->CreateVideoProcessorOutputView(slot->texture.Get(), out.enumerator.Get(), &view, &slot->view))) return hr;
            out.slots.push_back(slot);
        }
        out.width = f->width; out.height = f->height;
    }
    std::shared_ptr<DisplaySlot> slot;
    for (auto &candidate : out.slots) {
        bool available = false;
        if (candidate->busy.compare_exchange_strong(available, true)) { slot = candidate; break; }
    }
    // A slow UI must never accumulate latency or block the decoder.
    if (!slot) return S_FALSE;
    ComPtr<ID3D11VideoProcessorInputView> input;
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC view{};
    view.ViewDimension = D3D11_VPIV_DIMENSION_TEXTURE2D;
    view.Texture2D.ArraySlice = (UINT)(uintptr_t)f->data[1];
    hr = d3d->video_device->CreateVideoProcessorInputView((ID3D11Texture2D *)f->data[0], out.enumerator.Get(), &view, &input);
    if (FAILED(hr)) { slot->busy.store(false); return hr; }
    RECT rect{0, 0, f->width, f->height};
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE source{}, target{};
    source.YCbCr_Matrix = f->colorspace == AVCOL_SPC_BT709;
    source.Nominal_Range = f->color_range == AVCOL_RANGE_JPEG ? 2 : 1;
    target.Nominal_Range = 2;
    auto vc = d3d->video_context;
    d3d->lock(d3d->lock_ctx);
    vc->VideoProcessorSetStreamFrameFormat(out.processor.Get(), 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
    vc->VideoProcessorSetStreamAutoProcessingMode(out.processor.Get(), 0, FALSE);
    vc->VideoProcessorSetStreamSourceRect(out.processor.Get(), 0, TRUE, &rect);
    vc->VideoProcessorSetStreamDestRect(out.processor.Get(), 0, TRUE, &rect);
    vc->VideoProcessorSetOutputTargetRect(out.processor.Get(), TRUE, &rect);
    vc->VideoProcessorSetStreamColorSpace(out.processor.Get(), 0, &source);
    vc->VideoProcessorSetOutputColorSpace(out.processor.Get(), &target);
    D3D11_VIDEO_PROCESSOR_STREAM stream{};
    stream.Enable = TRUE; stream.pInputSurface = input.Get();
    hr = vc->VideoProcessorBlt(out.processor.Get(), slot->view.Get(), 0, 1, &stream);
    // Decoder and renderer submit to the same immediate context/command stream.
    d3d->unlock(d3d->lock_ctx);
    if (FAILED(hr)) { slot->busy.store(false); return hr; }
    *lease = new DisplayLease{slot};
    return S_OK;
}
extern "C" int ps_ff_frame(Codec *s, int *width, int *height, int *hardware,
                            int gpu_output, DisplayLease **lease, char *error, int cap) {
    *lease = NULL;
    av_frame_unref(s->frame); int result = avcodec_receive_frame(s->ctx, s->frame);
    if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) return 0;
    if (result < 0) { message(error, cap, "Decode output", result); return result; }
    // Decode dependencies, but only convert the newest available display frame.
    for (int i = 0; i < 7; ++i) {
        av_frame_unref(s->download);
        result = avcodec_receive_frame(s->ctx, s->download);
        if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) break;
        if (result < 0) { message(error, cap, "Decode output", result); return result; }
        av_frame_unref(s->frame); av_frame_move_ref(s->frame, s->download);
    }
    AVFrame *f = s->frame;
    if (f->width < 2 || f->height < 2 || f->width > 4096 || f->height > 4096 || (int64_t)f->width * f->height > 4096LL * 2160) return AVERROR(EINVAL);
    *width = f->width; *height = f->height; *hardware = f->format == AV_PIX_FMT_D3D11;
    if (gpu_output < 0) return 2; // Decode dependencies, skip obsolete display work.
    if (!gpu_output) s->display = DisplayConverter{};
    if (*hardware && gpu_output) {
        HRESULT hr = display_frame(s, f, lease);
        if (hr == S_OK) return 1;
        if (hr == S_FALSE) return 2;
        s->display = DisplayConverter{};
        snprintf(error, cap, "D3D11 display conversion unavailable (0x%08lx); using CPU output", (unsigned long)hr);
    }
    return 3; // CPU output needed; keep the received frame for ps_ff_rgba.
}
extern "C" int ps_ff_rgba(Codec *s, unsigned char *rgba, int capacity, char *error, int cap) {
    AVFrame *f = s->frame;
    int result;
    if (f->format == AV_PIX_FMT_D3D11) {
        av_frame_unref(s->download);
        if ((result = av_hwframe_transfer_data(s->download, f, 0)) < 0) { message(error, cap, "GPU decode download", result); return result; }
        f = s->download;
    }
    if (capacity < f->width * f->height * 4) { snprintf(error, cap, "Decode output exceeds declared stream dimensions"); return AVERROR(ENOSPC); }
    s->scale = sws_getCachedContext(s->scale, f->width, f->height, (AVPixelFormat)f->format,
                                  f->width, f->height, AV_PIX_FMT_RGBA, SWS_FAST_BILINEAR, NULL, NULL, NULL);
    if (!s->scale) return AVERROR(ENOMEM);
    uint8_t *output[4] = {rgba, NULL, NULL, NULL}; int lines[4] = {f->width * 4, 0, 0, 0};
    result = sws_scale(s->scale, f->data, f->linesize, 0, f->height, output, lines);
    if (result < 0) { message(error, cap, "Decode color conversion", result); return result; }
    return 1;
}
extern "C" unsigned ps_ff_version(void) { return avcodec_version(); }

struct GpuInfo { unsigned vendor; unsigned device; unsigned sub_system; unsigned revision; long long luid; long long driver; wchar_t name[128]; };
extern "C" int ps_ff_gpu(int index, GpuInfo *info) {
    IDXGIFactory1 *factory = NULL; IDXGIAdapter1 *adapter = NULL;
    if (FAILED(CreateDXGIFactory1(__uuidof(IDXGIFactory1), (void **)&factory))) return 0;
    HRESULT result = factory->EnumAdapters1(index, &adapter); factory->Release();
    if (FAILED(result)) return 0;
    DXGI_ADAPTER_DESC1 desc{}; result = adapter->GetDesc1(&desc);
    LARGE_INTEGER driver{}; adapter->CheckInterfaceSupport(__uuidof(IDXGIDevice), &driver); adapter->Release();
    if (FAILED(result)) return 0;
    info->vendor = desc.VendorId; info->device = desc.DeviceId; info->sub_system = desc.SubSysId; info->revision = desc.Revision;
    info->luid = ((long long)desc.AdapterLuid.HighPart << 32) | desc.AdapterLuid.LowPart;
    info->driver = driver.QuadPart; memcpy(info->name, desc.Description, sizeof(info->name));
    return 1;
}
extern "C" long long ps_ff_device_luid(ID3D11Device *device) {
    IDXGIDevice *dxgi = NULL; IDXGIAdapter *adapter = NULL; DXGI_ADAPTER_DESC desc{};
    if (FAILED(device->QueryInterface(__uuidof(IDXGIDevice), (void **)&dxgi))) return 0;
    HRESULT result = dxgi->GetAdapter(&adapter); dxgi->Release();
    if (FAILED(result)) return 0;
    result = adapter->GetDesc(&desc); adapter->Release();
    if (FAILED(result)) return 0;
    return ((long long)desc.AdapterLuid.HighPart << 32) | desc.AdapterLuid.LowPart;
}
extern "C" ID3D11Device *ps_ff_device(int index) {
    IDXGIFactory1 *factory = NULL; IDXGIAdapter1 *adapter = NULL;
    if (FAILED(CreateDXGIFactory1(__uuidof(IDXGIFactory1), (void **)&factory))) return NULL;
    HRESULT result = factory->EnumAdapters1(index, &adapter); factory->Release();
    if (FAILED(result)) return NULL;
    ID3D11Device *device = NULL;
    result = D3D11CreateDevice(adapter, D3D_DRIVER_TYPE_UNKNOWN, NULL, D3D11_CREATE_DEVICE_BGRA_SUPPORT, NULL, 0, D3D11_SDK_VERSION, &device, NULL, NULL);
    adapter->Release(); return SUCCEEDED(result) ? device : NULL;
}
extern "C" void ps_ff_drivers(char *out, int cap) {
    out[0] = 0; HKEY root;
    if (RegOpenKeyExA(HKEY_LOCAL_MACHINE, "SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}", 0, KEY_READ, &root) != ERROR_SUCCESS) return;
    int offset = 0;
    for (DWORD i = 0; i < 64; ++i) {
        char name[128]; DWORD length = sizeof(name);
        if (RegEnumKeyExA(root, i, name, &length, NULL, NULL, NULL, NULL) != ERROR_SUCCESS) break;
        HKEY entry; if (RegOpenKeyExA(root, name, 0, KEY_READ, &entry) != ERROR_SUCCESS) continue;
        for (const char *key : {"MatchingDeviceId", "DriverVersion", "DriverDate"}) {
            char data[256] = {}; DWORD size = sizeof(data) - 1;
            if (RegQueryValueExA(entry, key, NULL, NULL, (LPBYTE)data, &size) == ERROR_SUCCESS && offset + 300 < cap)
                offset += snprintf(out + offset, cap - offset, "%s=%s;", key, data);
        }
        RegCloseKey(entry);
    }
    RegCloseKey(root);
}

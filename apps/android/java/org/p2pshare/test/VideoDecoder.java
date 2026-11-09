package org.p2pshare.test;

import android.graphics.SurfaceTexture;
import android.media.*;
import android.os.*;
import android.view.Surface;
import java.nio.ByteBuffer;
import java.util.HashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/** Ordered, bounded MediaCodec input on a worker; OES texture updates stay on the EGL thread. */
final class VideoDecoder {
    private static final HashMap<String,VideoDecoder> decoders=new HashMap<>();
    private static final HandlerThread callbacks;
    private static final Handler handler;
    private static final Handler surfaceHandler;
    static {
        callbacks=new HandlerThread("p2pshare-video-decode");callbacks.start();handler=new Handler(callbacks.getLooper());
        HandlerThread surfaceCallbacks=new HandlerThread("p2pshare-video-ready");surfaceCallbacks.start();surfaceHandler=new Handler(surfaceCallbacks.getLooper());
    }
    private MediaCodec codec;
    private SurfaceTexture texture;
    private Surface surface;
    private final AtomicBoolean available=new AtomicBoolean(false);
    // Reserve the surface before releaseOutputBuffer: its callback is asynchronous.
    // At most one rendered buffer may await the EGL consumer.
    private final AtomicBoolean surfacePending=new AtomicBoolean(false);
    private int pendingOutput=-1;
    private volatile long offered,decoded,recoveries,coalesced,inputMisses;
    private final HashMap<Long,long[]> timings=new HashMap<>();
    private String decoderName;
    private final String mime;
    private static final HashMap<String,String> failures=new HashMap<>();
    private long pendingReceivedNs;
    private final AtomicLong surfaceReceivedNs=new AtomicLong(),displayLatencySumNs=new AtomicLong(),displayLatencyMaxNs=new AtomicLong(),displayLatencyCount=new AtomicLong();
    private volatile long renderStutters,lastRenderNs,maxRenderGapNs,expectedGapNs=33333333;
    private long previousPts=-1;
    private long previousDecoded,previousRendered,previousOffered,lastReportNs=System.nanoTime(),decodeSumNs,decodeMaxNs,decodeSamples;
    private volatile long rendered;
    private long started=SystemClock.elapsedRealtime();
    private final DecoderQueue inputs=new DecoderQueue();
    private final Runnable pumpTask=this::pump;
    private final Object scheduleLock=new Object();
    private boolean scheduled,recovery;
    private long scheduledAt;
    private String decoderMode;
    private volatile boolean closed;
    private int inFlight;
    private VideoDecoder(int name,int width,int height,String mime) throws Exception {
        this.mime=mime;
        try {
            texture=new SurfaceTexture(name);texture.setDefaultBufferSize(width,height);
            texture.setOnFrameAvailableListener(t -> { available.set(true);P2PShareActivity.nativeVideoReady(); },surfaceHandler);surface=new Surface(texture);
            Exception failure=null;
            for(int attempt=0;attempt<3;attempt++) {
                try {
                    codec=MediaCodec.createDecoderByType(mime);decoderName=codec.getName();
                    MediaFormat format=MediaFormat.createVideoFormat(mime,width,height);
                    format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE,2*1024*1024);
                    boolean generic=attempt<2 && Build.VERSION.SDK_INT>=30;
                    boolean exynos=attempt==0 && decoderName.toLowerCase(java.util.Locale.ROOT).contains("exynos");
                    if(generic) format.setInteger(MediaFormat.KEY_LOW_LATENCY,1);
                    if(exynos) format.setInteger("vendor.rtc-ext-dec-low-latency.enable",1);
                    codec.configure(format,surface,null,0);codec.start();
                    decoderMode=exynos?"exynos+generic":generic?"generic":"compatibility";
                    DebugLog.record("decoder_config mime="+mime+" codec="+decoderName+" mode="+decoderMode);
                    failure=null;break;
                } catch(Exception error) {
                    failure=error;DebugLog.record("decoder_config_retry attempt="+attempt+" error="+error);
                    if(codec!=null) { try {codec.release();} catch(Exception ignored) {} codec=null; }
                }
            }
            if(failure!=null) throw failure;
        } catch(Exception error) {
            if(codec!=null) { try { codec.release(); } catch(Exception ignored) {} }
            if(surface!=null) surface.release();if(texture!=null) texture.release();throw error;
        }
    }
    static synchronized boolean create(String peer,int texture,int w,int h,String mime) {
        remove(peer);
        try { failures.remove(peer);decoders.put(peer,new VideoDecoder(texture,w,h,mime));return true; }
        catch(Exception e) { failures.put(peer,"Cannot decode "+mime+": "+e);android.util.Log.e("P2PShare","Decoder creation",e);DebugLog.record("decoder_creation_error mime="+mime+" "+e);return false; }
    }
    private static synchronized VideoDecoder get(String peer) { return decoders.get(peer); }
    private void schedule(long delay) {
        synchronized(scheduleLock) {
            if(closed) return;
            long at=SystemClock.uptimeMillis()+delay;
            if(scheduled && at>=scheduledAt) return;
            if(scheduled) handler.removeCallbacks(pumpTask);
            scheduled=true;scheduledAt=at;handler.postAtTime(pumpTask,this,at);
        }
    }
    private void drain() {
        MediaCodec.BufferInfo info=new MediaCodec.BufferInfo();
        for(int i=0;i<12;i++) {
            int out=codec.dequeueOutputBuffer(info,0);
            if(out>=0) {
                inFlight=Math.max(0,inFlight-1);decoded++;
                long[] timing=timings.remove(info.presentationTimeUs);
                pendingReceivedNs=timing==null?0:timing[1];
                if(timing!=null) { long latency=System.nanoTime()-timing[0];decodeSumNs+=latency;decodeMaxNs=Math.max(decodeMaxNs,latency);decodeSamples++; }
                if(pendingOutput>=0) { codec.releaseOutputBuffer(pendingOutput,false);coalesced++; }
                pendingOutput=out;
            } else if(out!=MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) break;
        }
        if(pendingOutput>=0 && !closed && surfacePending.compareAndSet(false,true)) {
            int output=pendingOutput;pendingOutput=-1;
            surfaceReceivedNs.set(pendingReceivedNs);
            codec.releaseOutputBuffer(output,System.nanoTime());
        }
    }
    private synchronized void pump() {
        synchronized(scheduleLock) {scheduled=false;} if(closed) return;
        try {
            drain();
            for(int i=0;i<8 && !inputs.isEmpty();i++) {
                int index=codec.dequeueInputBuffer(0);
                if(index<0) { inputMisses++;break; } // Normal backpressure. Keep the compressed frame for the next pump.
                DecoderQueue.Input next=inputs.peek();ByteBuffer buffer=codec.getInputBuffer(index);
                if(buffer==null || buffer.capacity()<next.data.length) { codec.queueInputBuffer(index,0,0,next.micros,0);throw new IllegalStateException("Decoder input too small"); }
                if(timings.size()>=64) timings.clear();
                timings.put(next.micros,new long[]{System.nanoTime(),next.receivedNs});
                buffer.clear();buffer.put(next.data);codec.queueInputBuffer(index,0,next.data.length,next.micros,0);
                inputs.remove();inFlight++;
            }
            drain();report();if(!inputs.isEmpty() || inFlight>0 || pendingOutput>=0) schedule(3);
        } catch(Exception error) {
            android.util.Log.e("P2PShare","Decoder pump",error);DebugLog.record("decoder_error "+error);
            inputs.lost();timings.clear();inFlight=0;pendingOutput=-1;recovery=true;recoveries++;try { codec.flush(); } catch(Exception ignored) {}
        }
    }
    static boolean feed(String peer,byte[] data,long micros,boolean keyframe) { return feed(peer,data,micros,keyframe,0); }
    static boolean feed(String peer,byte[] data,long micros,boolean keyframe,long ageUs) { return feed(peer,data,micros,keyframe,ageUs,null); }
    static boolean feed(String peer,byte[] data,long micros,boolean keyframe,long ageUs,String mime) {
        VideoDecoder d=get(peer);if(d==null || (mime!=null && !d.mime.equals(mime))) return false;
        synchronized(d) {
            if(d.closed) return false;
            if(d.previousPts>=0 && micros>d.previousPts && micros-d.previousPts<=10000000) d.expectedGapNs=(micros-d.previousPts)*1000;
            d.previousPts=micros;
            boolean ok=d.inputs.offer(data,micros,keyframe,ageUs);d.offered++;if(!ok) d.recoveries++;
            boolean needsRecovery=d.recovery;d.recovery=false;
            if(!d.inputs.isEmpty()) d.schedule(0);
            return ok && !needsRecovery;
        }
    }
    static boolean update(String peer,float[] matrix) {
        VideoDecoder d=get(peer);if(d==null) return false;
        // Only the EGL thread touches SurfaceTexture. Do not take the codec lock:
        // the producer can be waiting for this consumer to free a surface buffer.
        if(d.closed || !d.available.getAndSet(false)) return false;
        try { d.texture.updateTexImage();d.texture.getTransformMatrix(matrix);d.rendered++;
            long now=System.nanoTime();if(d.lastRenderNs!=0) {long gap=now-d.lastRenderNs;d.maxRenderGapNs=Math.max(d.maxRenderGapNs,gap);if(gap>Math.max(100000000,d.expectedGapNs*5/2)) d.renderStutters++;} d.lastRenderNs=now;
            long received=d.surfaceReceivedNs.get();if(received!=0) {long latency=now-received;d.displayLatencyCount.incrementAndGet();d.displayLatencySumNs.addAndGet(latency);d.displayLatencyMaxNs.accumulateAndGet(latency,Math::max);}
            d.surfacePending.set(false);d.schedule(0);return true; }
        catch(Exception e) { d.surfacePending.set(false);android.util.Log.e("P2PShare","Decoder surface",e);return false; }
    }
    private void report() {
        long now=System.nanoTime();double seconds=(now-lastReportNs)/1e9;if(seconds<1) return;
        long displayCount=rendered-previousRendered;long latencyCount=displayLatencyCount.getAndSet(0);
        DebugLog.record(String.format(java.util.Locale.ROOT,
          "decoder codec=%s mode=%s mime=%s input_fps=%.2f decode_fps=%.2f display_fps=%.2f queue=%d drops=%d overflow=%d display_coalesced=%d input_misses=%d recoveries=%d decode_mean_ms=%.2f decode_max_ms=%.2f receive_display_mean_ms=%.2f receive_display_max_ms=%.2f stutters=%d max_display_gap_ms=%.2f stall_ms=%.2f waiting_idr=%s",
          decoderName,decoderMode,mime,(offered-previousOffered)/seconds,(decoded-previousDecoded)/seconds,displayCount/seconds,
          inputs.queued,inputs.dropped,inputs.overflows,coalesced,inputMisses,recoveries,
          decodeSamples==0?0:decodeSumNs/1e6/decodeSamples,decodeMaxNs/1e6,
          latencyCount==0?-1:displayLatencySumNs.getAndSet(0)/1e6/latencyCount,displayLatencyMaxNs.getAndSet(0)/1e6,
          renderStutters,maxRenderGapNs/1e6,lastRenderNs==0?0:(now-lastRenderNs)/1e6,inputs.waiting()));
        decodeSumNs=0;decodeMaxNs=0;decodeSamples=0;previousDecoded=decoded;previousRendered=rendered;previousOffered=offered;lastReportNs=now;
    }
    static synchronized String diagnostics() {
        StringBuilder text=new StringBuilder();
        for(String error:failures.values()) text.append(error).append("\n");
        for(VideoDecoder d:decoders.values()) {
            // Never block the EGL consumer on a codec monitor, including for logging.
            float seconds=Math.max(1f,(SystemClock.elapsedRealtime()-d.started)/1000f);
            text.append(String.format(java.util.Locale.ROOT,
              "%s [%s] • decoded %.1f FPS / displayed %.1f FPS • input %d • recovery %d • drops %d • stall %.0f ms%s\n",
              d.decoderName,d.decoderMode,d.decoded/seconds,d.rendered/seconds,d.offered,d.recoveries,d.inputs.dropped,d.lastRenderNs==0?0:(System.nanoTime()-d.lastRenderNs)/1e6,
              d.inputs.waiting()?" • waiting for keyframe":""));
        }
        return text.toString();
    }
    static synchronized void remove(String peer) {
        VideoDecoder d=decoders.remove(peer);if(d==null) return;
        d.closed=true;handler.removeCallbacksAndMessages(d);
        // Abandon the surface first so a blocked producer cannot prevent teardown.
        d.texture.setOnFrameAvailableListener(null);d.surface.release();d.texture.release();
        synchronized(d) {
            d.inputs.lost();
            try { d.codec.stop(); } catch(Exception ignored) {}
            d.codec.release();
        }
    }
}

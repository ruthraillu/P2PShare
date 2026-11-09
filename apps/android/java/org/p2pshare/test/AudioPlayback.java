package org.p2pshare.test;

import android.content.Context;
import android.media.*;
import android.os.Handler;
import android.os.Looper;

/** AudioTrack only handles stereo PCM. Opus decoding and mixing live in Rust. */
final class AudioPlayback {
    private final Context context;
    private AudioTrack track;
    private AudioManager manager;
    private AudioFocusRequest focusRequest;
    private volatile boolean focus = true;
    private long written;
    private long headWrap;
    private long lastHead;
    private long underruns;
    private long shortWrites;
    private long queued;
    private int previousUnderruns;
    AudioPlayback(Context context) { this.context=context; }
    int available() {
        try {
            if(track==null) {
                AudioAttributes attributes=new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MOVIE).build();
                manager=(AudioManager)context.getSystemService(Context.AUDIO_SERVICE);
                focusRequest=new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN).setAudioAttributes(attributes).setOnAudioFocusChangeListener(change->{focus=change==AudioManager.AUDIOFOCUS_GAIN;},new Handler(Looper.getMainLooper())).build();
                focus=manager.requestAudioFocus(focusRequest)==AudioManager.AUDIOFOCUS_REQUEST_GRANTED;
                int minimum=AudioTrack.getMinBufferSize(48000,AudioFormat.CHANNEL_OUT_STEREO,AudioFormat.ENCODING_PCM_16BIT);
                if(minimum<=0) throw new IllegalStateException("Unsupported PCM audio");
                track=new AudioTrack.Builder().setAudioAttributes(attributes).setAudioFormat(new AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).build()).setTransferMode(AudioTrack.MODE_STREAM).setBufferSizeInBytes(Math.max(minimum,3840)).setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY).build();
                if(track.getState()!=AudioTrack.STATE_INITIALIZED) throw new IllegalStateException("AudioTrack init failed");
                written=0;lastHead=0;headWrap=0;previousUnderruns=0;
                track.play();
                DebugLog.record("audio_track_started rate=48000 channels=2 device_buffer_frames="+track.getBufferSizeInFrames());
            }
            if(!focus) {if(track.getPlayState()==AudioTrack.PLAYSTATE_PLAYING){track.pause();track.flush();written=0;lastHead=0;headWrap=0;}return 0;}
            if(track.getPlayState()!=AudioTrack.PLAYSTATE_PLAYING) track.play();
            long head=Integer.toUnsignedLong(track.getPlaybackHeadPosition());
            if(head<lastHead) headWrap+=1L<<32;
            lastHead=head;
            queued=Math.max(0,written-head-headWrap);
            int current=track.getUnderrunCount();
            underruns+=Math.max(0,current-previousUnderruns);previousUnderruns=current;
            // Only queue 10–20 ms even if the driver allocates a larger buffer.
            return queued<=480?(int)Math.min(480,track.getBufferSizeInFrames()-queued):0;
        } catch(RuntimeException error) {DebugLog.record("audio_output_error="+error.getClass().getSimpleName());close();return -1;}
    }
    int write(short[] samples) {
        if(track==null||!focus) return 0;
        try {
            int count=track.write(samples,0,samples.length,AudioTrack.WRITE_NON_BLOCKING);
            if(count<0){DebugLog.record("audio_track_write_error="+count);return -1;}
            written+=count/2;
            if(count<samples.length) shortWrites++;
            return count;
        }catch(RuntimeException error){DebugLog.record("audio_write_error="+error.getClass().getSimpleName());return -1;}
    }
    String diagnostics(){return "audio_device queued_ms="+(queued/48.0)+" underruns="+underruns+" short_writes="+shortWrites+" focus="+focus+"\n";}
    void close() {
        if(track!=null){try{track.pause();track.flush();}catch(RuntimeException ignored){}track.release();track=null;}
        if(manager!=null&&focusRequest!=null){manager.abandonAudioFocusRequest(focusRequest);focusRequest=null;}
        queued=0;
    }
}

package org.p2pshare.test;

import android.app.*;
import android.content.*;
import android.content.pm.ServiceInfo;
import android.graphics.*;
import android.hardware.display.*;
import android.media.*;
import android.media.projection.*;
import android.opengl.*;
import android.os.*;
import android.view.Surface;
import java.nio.*;
import java.io.ByteArrayOutputStream;

/** User-authorized projection -> GPU texture -> hardware AVC. No activity/UI dependency. */
public final class ScreenShareService extends Service {
    private static volatile ScreenShareService activeService;
    private volatile boolean switchPending;
    private boolean projectionEnded;
    private MediaProjection.Callback projectionCallback;
    public static void beginSwitch(){ScreenShareService s=activeService;if(s!=null)s.switchPending=true;}
    public static void cancelSwitch(){ScreenShareService s=activeService;if(s!=null&&s.worker!=null)s.worker.post(()->{s.switchPending=false;if(s.projectionEnded)s.stopSelf();});}
    private HandlerThread thread;
    private Handler worker;
    private MediaProjection projection;
    private VirtualDisplay display;
    private MediaCodec encoder;
    private Surface codecSurface,captureSurface;
    private android.graphics.SurfaceTexture texture;
    private EGLDisplay egl;
    private EGLContext context;
    private EGLConfig config;
    private EGLSurface window;
    private int tex,program,width,height,density;
    private boolean stopped,available,hasFrame;
    private int fps=30,bitrate=6000,maxEdge=1280;
    private String mime="video/avc";
    private volatile boolean audioStopped;
    private AudioRecord audioRecord;
    private Thread audioThread;
    public static native void nativeConfigure(boolean hevc,int bitrate,int opus);
    public static native void nativeAudio(short[] samples);
    private byte[] headers=new byte[0];
    private long lastPreview,nextFrameAt;
    private final float[] matrix=new float[16];
    private FloatBuffer vertices;
    private final MediaCodec.BufferInfo info=new MediaCodec.BufferInfo();
    public static native void nativeSharing(boolean active,int width,int height,String error);
    public static native void nativeFrame(byte[] bytes,boolean keyframe);
    public static native void nativePreview(byte[] jpeg);
    public static native int nativeDemand(); // -1 left room, 0 preview-only, 1 live, 2 keyframe
    @Override public int onStartCommand(Intent intent,int flags,int startId) {
        L10n.init(this);
        if(intent==null||"stop".equals(intent.getAction())){stopSelf();return START_NOT_STICKY;}
        if(thread!=null){
            final int code=intent.getIntExtra("result",Activity.RESULT_CANCELED);
            final Intent token=Build.VERSION.SDK_INT>=33?intent.getParcelableExtra("data",Intent.class):intent.getParcelableExtra("data");
            worker.post(()->{try{worker.removeCallbacks(tick);releaseCapture();stopped=false;audioStopped=false;available=false;hasFrame=false;width=0;height=0;nextFrameAt=0;loadSettings();start(code,token);}catch(Exception e){fail(e);}});
            return START_NOT_STICKY;
        }
        activeService=this;
        NotificationManager nm=(NotificationManager)getSystemService(NOTIFICATION_SERVICE);
        nm.createNotificationChannel(new NotificationChannel("screen_share",L10n.t("Screen sharing"),NotificationManager.IMPORTANCE_LOW));
        PendingIntent stop=PendingIntent.getService(this,22,new Intent(this,ScreenShareService.class).setAction("stop"),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        PendingIntent open=PendingIntent.getActivity(this,23,new Intent(this,P2PShareActivity.class),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        Notification notification=new Notification.Builder(this,"screen_share").setSmallIcon(android.R.drawable.ic_menu_view).setContentTitle(L10n.t("Sharing your screen"))
            .setContentText(L10n.t("Tap Stop sharing to end video and audio capture")).setContentIntent(open).setOngoing(true).addAction(new Notification.Action.Builder(null,L10n.t("Stop sharing"),stop).build()).build();
        if(Build.VERSION.SDK_INT>=29)startForeground(8,notification,ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION);else startForeground(8,notification);
        loadSettings();
        thread=new HandlerThread("p2pshare-capture");thread.start();worker=new Handler(thread.getLooper());
        final int result=intent.getIntExtra("result",Activity.RESULT_CANCELED);
        final Intent data=Build.VERSION.SDK_INT>=33?intent.getParcelableExtra("data",Intent.class):intent.getParcelableExtra("data");
        worker.post(()->{try{start(result,data);}catch(Exception e){fail(e);}});
        return START_NOT_STICKY;
    }
    private void loadSettings(){
        android.content.SharedPreferences prefs=getSharedPreferences("sharing",0);
        fps=new int[]{30,60,90,120}[Math.max(0,Math.min(3,prefs.getInt("fps",0)))];
        maxEdge=new int[]{854,1280,1920,2560,3840}[Math.max(0,Math.min(4,prefs.getInt("res",1)))];
        bitrate=Math.max(100,Math.min(50000,prefs.getInt("bitrate",6000)));mime=prefs.getInt("codec",0)==1?"video/hevc":"video/avc";
        nativeConfigure(mime.equals("video/hevc"),bitrate,Math.max(64,Math.min(512,prefs.getInt("opus",192))));
    }
    private void start(int result,Intent data)throws Exception {
        projection=((MediaProjectionManager)getSystemService(MEDIA_PROJECTION_SERVICE)).getMediaProjection(result,data);
        if(projection==null)throw new IllegalStateException("Screen capture permission unavailable");
        final MediaProjection current=projection;
        projectionCallback=new MediaProjection.Callback(){
            @Override public void onStop(){if(projection!=current)return;projectionEnded=true;if(!switchPending)stopSelf();}
            @Override public void onCapturedContentResize(int w,int h){if(projection==current&&!stopped&&w>0&&h>0)try{resize(w,h);}catch(Exception e){fail(e);}}
        };projection.registerCallback(projectionCallback,worker);projectionEnded=false;switchPending=false;
        android.util.DisplayMetrics metrics=getResources().getDisplayMetrics();density=metrics.densityDpi;
        initGl();int[] ids=new int[1];GLES20.glGenTextures(1,ids,0);tex=ids[0];GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,tex);
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_MIN_FILTER,GLES20.GL_LINEAR);GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_MAG_FILTER,GLES20.GL_LINEAR);
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_WRAP_S,GLES20.GL_CLAMP_TO_EDGE);GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_WRAP_T,GLES20.GL_CLAMP_TO_EDGE);
        texture=new android.graphics.SurfaceTexture(tex);texture.setOnFrameAvailableListener(t->{available=true;},worker);captureSurface=new Surface(texture);
        resize(metrics.widthPixels,metrics.heightPixels);
        display=projection.createVirtualDisplay("P2PShare",width,height,density,DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,captureSurface,null,worker);
        startAudio();worker.post(tick);
    }
    private void resize(int w,int h)throws Exception {
        float scale=Math.min(1f,(float)maxEdge/Math.max(w,h));int nw=Math.max(64,Math.round(w*scale)/2*2),nh=Math.max(64,Math.round(h*scale)/2*2);
        if(nw==width&&nh==height)return;
        closeEncoder();width=nw;height=nh;headers=new byte[0];lastPreview=0;hasFrame=false;
        MediaFormat format=MediaFormat.createVideoFormat(mime,width,height);
        format.setInteger(MediaFormat.KEY_COLOR_FORMAT,MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface);
        format.setInteger(MediaFormat.KEY_BIT_RATE,bitrate*1000);format.setInteger(MediaFormat.KEY_FRAME_RATE,fps);format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL,2);
        if(mime.equals("video/avc"))format.setInteger(MediaFormat.KEY_PROFILE,MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline);
        encoder=MediaCodec.createEncoderByType(mime);encoder.configure(format,null,null,MediaCodec.CONFIGURE_FLAG_ENCODE);codecSurface=encoder.createInputSurface();encoder.start();
        window=EGL14.eglCreateWindowSurface(egl,config,codecSurface,new int[]{EGL14.EGL_NONE},0);makeCurrent();
        texture.setDefaultBufferSize(width,height);if(display!=null)display.resize(width,height,density);
        nativeSharing(true,width,height,"");DebugLog.record("capture_started width="+width+" height="+height+" encoder="+encoder.getName()+" codec="+mime+" bitrate_kbps="+bitrate+" fps="+fps+"");
    }
    private final Runnable tick=new Runnable(){public void run(){if(stopped)return;try{
        int demand=nativeDemand();if(demand<0){stopSelf();return;}
        if(projectionEnded){worker.postDelayed(this,100);return;}
        makeCurrent();drain();
        if(available){available=false;texture.updateTexImage();texture.getTransformMatrix(matrix);hasFrame=true;}
        boolean preview=SystemClock.elapsedRealtime()-lastPreview>=10000;
        if(hasFrame&&(demand>0||preview)){
            if(demand==2){Bundle b=new Bundle();b.putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME,0);encoder.setParameters(b);}
            GLES20.glViewport(0,0,width,height);GLES20.glUseProgram(program);GLES20.glActiveTexture(GLES20.GL_TEXTURE0);GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,tex);
            int a=GLES20.glGetAttribLocation(program,"a");vertices.position(0);GLES20.glEnableVertexAttribArray(a);GLES20.glVertexAttribPointer(a,2,GLES20.GL_FLOAT,false,0,vertices);
            GLES20.glUniformMatrix4fv(GLES20.glGetUniformLocation(program,"m"),1,false,matrix,0);GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP,0,4);
            if(preview){preview();lastPreview=SystemClock.elapsedRealtime();}
            EGLExt.eglPresentationTimeANDROID(egl,window,System.nanoTime());if(!EGL14.eglSwapBuffers(egl,window))throw new IllegalStateException("Encoder surface lost");drain();
        }
        long now=System.nanoTime();if(nextFrameAt==0)nextFrameAt=now;nextFrameAt+=1000000000L/fps;if(nextFrameAt<now)nextFrameAt=now;
        worker.postDelayed(this,Math.max(1,(nextFrameAt-System.nanoTime()+999999)/1000000));
    }catch(Exception e){fail(e);}}};
    private void startAudio(){
        if(Build.VERSION.SDK_INT<29||!getSharedPreferences("sharing",0).getBoolean("audio",true))return;
        try{
            AudioPlaybackCaptureConfiguration.Builder capture=new AudioPlaybackCaptureConfiguration.Builder(projection).addMatchingUsage(AudioAttributes.USAGE_MEDIA).addMatchingUsage(AudioAttributes.USAGE_GAME).addMatchingUsage(AudioAttributes.USAGE_UNKNOWN);
            int uid=getSharedPreferences("sharing",0).getInt("uid",-1);if(uid>=0)capture.addMatchingUid(uid);else capture.excludeUid(getApplicationInfo().uid);
            AudioFormat format=new AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_IN_STEREO).build();
            int buffer=Math.max(19200,AudioRecord.getMinBufferSize(48000,AudioFormat.CHANNEL_IN_STEREO,AudioFormat.ENCODING_PCM_16BIT));
            audioRecord=new AudioRecord.Builder().setAudioFormat(format).setBufferSizeInBytes(buffer).setAudioPlaybackCaptureConfig(capture.build()).build();
            if(audioRecord.getState()!=AudioRecord.STATE_INITIALIZED)throw new IllegalStateException("Playback audio capture unavailable");
            audioRecord.startRecording();final AudioRecord input=audioRecord;audioThread=new Thread(()->{android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO);short[] pcm=new short[960];int offset=0;
                try{while(!audioStopped){int n=input.read(pcm,offset,pcm.length-offset,AudioRecord.READ_BLOCKING);if(n<=0)break;offset+=n;if(offset==pcm.length){nativeAudio(pcm);offset=0;}}}
                catch(Exception e){DebugLog.record("audio_capture_error="+e);}
                finally{input.release();}
            },"p2pshare-audio-capture");audioThread.start();DebugLog.record("audio_capture_started uid="+uid+" rate=48000 stereo=true");
        }catch(Exception e){if(audioRecord!=null){audioRecord.release();audioRecord=null;}DebugLog.record("audio_capture_unavailable="+e);new Handler(Looper.getMainLooper()).post(()->android.widget.Toast.makeText(this,L10n.t("Video sharing started, but audio capture is unavailable"),android.widget.Toast.LENGTH_LONG).show());}
    }
    private void drain(){
        for(int n=0;n<16;n++){
            int index=encoder.dequeueOutputBuffer(info,0);
            if(index==MediaCodec.INFO_OUTPUT_FORMAT_CHANGED){MediaFormat f=encoder.getOutputFormat();ByteArrayOutputStream b=new ByteArrayOutputStream();for(String key:new String[]{"csd-0","csd-1"}){ByteBuffer data=f.getByteBuffer(key);if(data!=null){byte[] bytes=new byte[data.remaining()];data.duplicate().get(bytes);b.write(bytes,0,bytes.length);}}headers=b.toByteArray();continue;}
            if(index<0)break;
            try{ByteBuffer data=encoder.getOutputBuffer(index);if(data!=null&&info.size>0){data.position(info.offset);data.limit(info.offset+info.size);byte[] bytes=new byte[info.size];data.get(bytes);
                if((info.flags&MediaCodec.BUFFER_FLAG_CODEC_CONFIG)!=0)headers=bytes;
                else{boolean key=(info.flags&MediaCodec.BUFFER_FLAG_KEY_FRAME)!=0;if(key&&headers.length>0){byte[] combined=new byte[headers.length+bytes.length];System.arraycopy(headers,0,combined,0,headers.length);System.arraycopy(bytes,0,combined,headers.length,bytes.length);bytes=combined;}nativeFrame(bytes,key);}
            }}finally{encoder.releaseOutputBuffer(index,false);}
        }
    }
    private void preview(){
        // Read back at most 512px per edge; video stays entirely on the GPU.
        int pw=Math.max(1,Math.round(width* Math.min(1f,512f/Math.max(width,height))));int ph=Math.max(1,Math.round(height*Math.min(1f,512f/Math.max(width,height))));
        GLES20.glViewport(0,0,pw,ph);GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP,0,4);
        ByteBuffer pixels=ByteBuffer.allocateDirect(pw*ph*4);GLES20.glReadPixels(0,0,pw,ph,GLES20.GL_RGBA,GLES20.GL_UNSIGNED_BYTE,pixels);
        Bitmap raw=Bitmap.createBitmap(pw,ph,Bitmap.Config.ARGB_8888);raw.copyPixelsFromBuffer(pixels);android.graphics.Matrix flip=new android.graphics.Matrix();flip.setScale(1,-1);Bitmap image=Bitmap.createBitmap(raw,0,0,pw,ph,flip,true);
        ByteArrayOutputStream out=new ByteArrayOutputStream();for(int quality:new int[]{65,50,35,20,10,1}){out.reset();image.compress(Bitmap.CompressFormat.JPEG,quality,out);if(out.size()<=48*1024)break;}if(out.size()<=48*1024)nativePreview(out.toByteArray());image.recycle();if(image!=raw)raw.recycle();
        // Restore the full encoder frame after drawing the thumbnail.
        GLES20.glViewport(0,0,width,height);GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP,0,4);
    }
    private int shader(int type,String source){int s=GLES20.glCreateShader(type);GLES20.glShaderSource(s,source);GLES20.glCompileShader(s);int[] ok=new int[1];GLES20.glGetShaderiv(s,GLES20.GL_COMPILE_STATUS,ok,0);if(ok[0]==0)throw new IllegalStateException(GLES20.glGetShaderInfoLog(s));return s;}
    private void initGl(){
        egl=EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY);if(!EGL14.eglInitialize(egl,new int[2],0,new int[2],0))throw new IllegalStateException("EGL unavailable");
        EGLConfig[] configs=new EGLConfig[1];int[] count=new int[1];EGL14.eglChooseConfig(egl,new int[]{EGL14.EGL_RED_SIZE,8,EGL14.EGL_GREEN_SIZE,8,EGL14.EGL_BLUE_SIZE,8,EGL14.EGL_RENDERABLE_TYPE,EGL14.EGL_OPENGL_ES2_BIT,EGL14.EGL_SURFACE_TYPE,EGL14.EGL_WINDOW_BIT|EGL14.EGL_PBUFFER_BIT,0x3142,1,EGL14.EGL_NONE},0,configs,0,1,count,0);if(count[0]==0)throw new IllegalStateException("No recordable EGL config");config=configs[0];
        context=EGL14.eglCreateContext(egl,config,EGL14.EGL_NO_CONTEXT,new int[]{EGL14.EGL_CONTEXT_CLIENT_VERSION,2,EGL14.EGL_NONE},0);
        window=EGL14.eglCreatePbufferSurface(egl,config,new int[]{EGL14.EGL_WIDTH,1,EGL14.EGL_HEIGHT,1,EGL14.EGL_NONE},0);makeCurrent();
        program=GLES20.glCreateProgram();int v=shader(GLES20.GL_VERTEX_SHADER,"attribute vec2 a;uniform mat4 m;varying vec2 uv;void main(){gl_Position=vec4(a,0.,1.);uv=(m*vec4((a+1.)*.5,0.,1.)).xy;}");int f=shader(GLES20.GL_FRAGMENT_SHADER,"#extension GL_OES_EGL_image_external : require\nprecision mediump float;uniform samplerExternalOES t;varying vec2 uv;void main(){gl_FragColor=texture2D(t,uv);}");GLES20.glAttachShader(program,v);GLES20.glAttachShader(program,f);GLES20.glLinkProgram(program);int[] ok=new int[1];GLES20.glGetProgramiv(program,GLES20.GL_LINK_STATUS,ok,0);if(ok[0]==0)throw new IllegalStateException("Capture shader linking failed");GLES20.glDeleteShader(v);GLES20.glDeleteShader(f);
        vertices=ByteBuffer.allocateDirect(32).order(ByteOrder.nativeOrder()).asFloatBuffer();vertices.put(new float[]{-1,-1,1,-1,-1,1,1,1}).position(0);
    }
    private void makeCurrent(){if(!EGL14.eglMakeCurrent(egl,window,window,context))throw new IllegalStateException("Cannot bind capture EGL context");}
    private void fail(Exception e){new Handler(Looper.getMainLooper()).post(()->android.widget.Toast.makeText(this,"Screen sharing failed: "+e.getMessage(),android.widget.Toast.LENGTH_LONG).show());DebugLog.record("capture_error="+e);nativeSharing(false,0,0,e.toString());stopSelf();}
    private void closeEncoder(){if(egl!=null&&window!=null){EGL14.eglMakeCurrent(egl,EGL14.EGL_NO_SURFACE,EGL14.EGL_NO_SURFACE,EGL14.EGL_NO_CONTEXT);EGL14.eglDestroySurface(egl,window);window=null;}if(encoder!=null){try{encoder.stop();}catch(Exception ignored){}encoder.release();encoder=null;}if(codecSurface!=null){codecSurface.release();codecSurface=null;}}
    private void releaseCapture(){
        audioStopped=true;if(audioRecord!=null&&audioThread!=null){try{audioRecord.stop();}catch(Exception ignored){}try{audioThread.join(1000);}catch(InterruptedException e){Thread.currentThread().interrupt();}audioThread=null;audioRecord=null;}
        if(projection!=null&&projectionCallback!=null)projection.unregisterCallback(projectionCallback);
        if(display!=null){display.release();display=null;}if(projection!=null){MediaProjection old=projection;projection=null;old.stop();}
        if(texture!=null){texture.setOnFrameAvailableListener(null);texture.release();texture=null;}if(captureSurface!=null){captureSurface.release();captureSurface=null;}closeEncoder();if(egl!=null){EGL14.eglDestroyContext(egl,context);EGL14.eglTerminate(egl);egl=null;}
    }
    @Override public void onDestroy(){if(activeService==this)activeService=null;if(worker!=null)worker.post(()->{
        stopped=true;worker.removeCallbacks(tick);releaseCapture();nativeSharing(false,0,0,"");thread.quitSafely();
    });super.onDestroy();}
    @Override public void onTaskRemoved(Intent intent){stopSelf();}
    @Override public IBinder onBind(Intent intent){return null;}
}

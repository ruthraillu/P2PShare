package org.p2pshare.test;
import android.os.Handler; import android.graphics.SurfaceTexture; import android.media.MediaCodec;
public final class VideoDecoderTest {
 public static void main(String[] args) {
  assert VideoDecoder.create("test",1,1280,720,"video/avc");byte[] key={1},p={2};float[] matrix=new float[16];
  MediaCodec.busy=true;assert VideoDecoder.feed("test",key,0,true);Handler.runOne();
  assert SurfaceTexture.last.outstanding==0; // input backpressure retains the IDR
  MediaCodec.busy=false;Handler.runOne();assert SurfaceTexture.last.outstanding==1;
  // Deliberately delay frame-available callbacks. The real callback is asynchronous.
  assert VideoDecoder.feed("test",p,33333,false);Handler.runOne();
  assert VideoDecoder.feed("test",p,66666,false);Handler.runOne();
  assert SurfaceTexture.last.outstanding==1;assert MediaCodec.dropped==1;
  assert !VideoDecoder.update("test",matrix);
  SurfaceTexture.last.signal();assert VideoDecoder.update("test",matrix);Handler.runOne();
  assert SurfaceTexture.last.outstanding==1;SurfaceTexture.last.signal();assert VideoDecoder.update("test",matrix);
  assert SurfaceTexture.last.acquired==2;
  assert VideoDecoder.diagnostics().contains("recovery 0");
  assert MediaCodec.lastReleaseNs>0;
  VideoDecoder.remove("test");
  MediaCodec.rejectVendor=true;MediaCodec.configureCalls=0;MediaCodec.releases=0;
  assert VideoDecoder.create("fallback",2,1280,720,"video/avc");
  assert MediaCodec.configureCalls==2 && MediaCodec.releases==1;
  assert VideoDecoder.diagnostics().contains("[generic]");VideoDecoder.remove("fallback");
  MediaCodec.rejectGeneric=true;MediaCodec.configureCalls=0;MediaCodec.releases=0;
  assert VideoDecoder.create("compatibility",3,1280,720,"video/avc");
  assert MediaCodec.configureCalls==3 && MediaCodec.releases==2;
  assert VideoDecoder.diagnostics().contains("[compatibility]");VideoDecoder.remove("compatibility");
  MediaCodec.rejectVendor=false;MediaCodec.rejectGeneric=false;
  for(String mime:new String[]{"video/hevc"}) {
   assert VideoDecoder.create("modern",4,1280,720,mime);assert MediaCodec.lastMime.equals(mime);
   assert !VideoDecoder.feed("modern",key,0,true,0,"video/avc");
   assert VideoDecoder.feed("modern",key,0,true);Handler.runOne();SurfaceTexture.last.signal();assert VideoDecoder.update("modern",matrix);
   VideoDecoder.remove("modern");
  }
  System.out.println("Decoder pump: retained busy input, delayed callbacks, bounded surface handoff, timed release and Exynos/generic compatibility fallback passed");
 }
}

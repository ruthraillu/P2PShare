package android.view; import android.graphics.SurfaceTexture; public class Surface { public final SurfaceTexture texture; public Surface(SurfaceTexture t){texture=t;} public void release(){} }

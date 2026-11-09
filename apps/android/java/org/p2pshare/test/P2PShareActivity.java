package org.p2pshare.test;

import android.app.NativeActivity;
import android.content.*;
import android.os.Bundle;
import android.os.Build;
import android.view.WindowManager;
import android.content.ClipboardManager;
import android.content.ClipData;

/** Minimal Java shell for Android consent, clipboard and platform codecs; the UI is Rust egui. */
public final class P2PShareActivity extends NativeActivity {
    private final AudioPlayback audioPlayback=new AudioPlayback(this);
    private boolean updateCheckStarted;

    private static native void nativeReleaseMetadata(String metadata);

    public synchronized void checkForUpdate() {
        if (updateCheckStarted) return;
        updateCheckStarted = true;
        new Thread(() -> {
            try {
                String metadata = ReleaseUpdates.fetchLatest();
                if (metadata != null) nativeReleaseMetadata(metadata);
            } catch (java.io.IOException error) {
                DebugLog.record("android_update_check_failed=" + error.getClass().getSimpleName());
            }
        }, "p2pshare-update-check").start();
    }

    public void openUpdatePage(String url) {
        runOnUiThread(() -> {
            try {
                startActivity(new Intent(Intent.ACTION_VIEW, android.net.Uri.parse(url)));
            } catch (ActivityNotFoundException error) {
                android.widget.Toast.makeText(this, L10n.t("Could not open the release page."),
                    android.widget.Toast.LENGTH_SHORT).show();
            }
        });
    }
    public int audioAvailable(){return audioPlayback.available();}
    public int writeAudio(short[] samples){return audioPlayback.write(samples);}
    public void closeAudio(){audioPlayback.close();}
    public String audioDiagnostics(){return audioPlayback.diagnostics();}
    public void startScreenShare(){runOnUiThread(()->ShareSettings.show(this));}
    void requestShareConsent(){
        if(getSharedPreferences("sharing",0).getBoolean("audio",true)&&Build.VERSION.SDK_INT>=29&&checkSelfPermission(android.Manifest.permission.RECORD_AUDIO)!=android.content.pm.PackageManager.PERMISSION_GRANTED){requestPermissions(new String[]{android.Manifest.permission.RECORD_AUDIO},96);return;}
        openShareConsent();
    }
    @Override public void onRequestPermissionsResult(int request,String[] permissions,int[] results){super.onRequestPermissionsResult(request,permissions,results);if(request==96){if(results.length==0||results[0]!=android.content.pm.PackageManager.PERMISSION_GRANTED)getSharedPreferences("sharing",0).edit().putBoolean("audio",false).apply();openShareConsent();}}
    private void openShareConsent(){runOnUiThread(()->{
        ScreenShareService.beginSwitch();
        try{startActivityForResult(((android.media.projection.MediaProjectionManager)getSystemService(MEDIA_PROJECTION_SERVICE)).createScreenCaptureIntent(),95);}
        catch(Exception e){ScreenShareService.cancelSwitch();}
    });}
    public void stopScreenShare(){stopService(new Intent(this,ScreenShareService.class));}
    @Override protected void onActivityResult(int request,int result,Intent data){
        super.onActivityResult(request,result,data);
        if(request==96){if(result==RESULT_OK&&data!=null&&data.getData()!=null)ProfileImages.load(this,data.getData());return;}
        if(request==95&&(result!=RESULT_OK||data==null)){ScreenShareService.cancelSwitch();return;}
        if(request==95&&result==RESULT_OK&&data!=null){
            try{startForegroundService(new Intent(this,ScreenShareService.class).putExtra("result",result).putExtra("data",data));}
            catch(Exception e){ScreenShareService.nativeSharing(false,0,0,e.toString());}
        }
    }
    public static native void nativeAvatarPicked(byte[] data);
    public void chooseAvatar(){runOnUiThread(()->{try{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT);pick.addCategory(Intent.CATEGORY_OPENABLE);pick.setType("image/*");startActivityForResult(pick,96);}catch(Exception e){android.widget.Toast.makeText(this,L10n.t("Image picker unavailable"),android.widget.Toast.LENGTH_LONG).show();}});}
    private boolean roomActive;
    private boolean notificationAsked;
    public static native void nativeLeaveRoom();
    public static native void nativeForeground(boolean active);
    public static native void nativeRoomEdited(String room);
    public static native String nativeSaveConfig(String text);
    private volatile boolean pipAvailable;
    private boolean pipSession;
    private volatile int pipWidth=16,pipHeight=9;
    public static native void nativePipMode(boolean active);
    public static native void nativePipClosed();
    @Override public void onStart(){super.onStart();nativeForeground(true);}
    @Override public void onResume(){super.onResume();if(!isInPictureInPictureMode())pipSession=false;nativeForeground(true);}
    @Override public void onPause(){super.onPause();}
    @Override public void onStop(){
        super.onStop();nativeForeground(false);
        if(pipSession || isInPictureInPictureMode()){pipSession=false;nativePipClosed();}
    }
    public void updatePip(boolean available,int width,int height){
        if(pipAvailable==available && pipWidth==width && pipHeight==height)return;
        runOnUiThread(()->{
        if(pipAvailable==available && pipWidth==width && pipHeight==height)return;
        pipAvailable=available;pipWidth=Math.max(1,width);pipHeight=Math.max(1,height);
        if(getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_PICTURE_IN_PICTURE)){
            try{setPictureInPictureParams(pipParams());}catch(RuntimeException e){DebugLog.record("pip_params_error="+e);}
        }
        if(!available && isInPictureInPictureMode()){moveTaskToBack(true);}
    });}
    private android.app.PictureInPictureParams pipParams(){
        // Android's normal PiP aspect-ratio range; portrait/ultrawide streams are fitted.
        float ratio=Math.max(0.42f,Math.min(2.38f,(float)pipWidth/pipHeight));
        android.app.PictureInPictureParams.Builder b=new android.app.PictureInPictureParams.Builder()
            .setAspectRatio(new android.util.Rational(Math.round(ratio*10000),10000));
        if(Build.VERSION.SDK_INT>=31)b.setAutoEnterEnabled(pipAvailable).setSeamlessResizeEnabled(true);
        return b.build();
    }
    public void enterPip(){runOnUiThread(()->{
        if(!pipAvailable)return;
        try{
            if(!getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_PICTURE_IN_PICTURE)
                || !enterPictureInPictureMode(pipParams()))android.widget.Toast.makeText(this,L10n.t("Picture-in-picture is unavailable. Check Android app permissions."),android.widget.Toast.LENGTH_LONG).show();
        }catch(RuntimeException e){DebugLog.record("pip_enter_error="+e);android.widget.Toast.makeText(this,L10n.t("Enable picture-in-picture for P2PShare in Android settings."),android.widget.Toast.LENGTH_LONG).show();}
    });}
    @Override protected void onUserLeaveHint(){super.onUserLeaveHint();if(Build.VERSION.SDK_INT<31&&pipAvailable)enterPip();}
    @Override public void onPictureInPictureModeChanged(boolean active,android.content.res.Configuration config){
        super.onPictureInPictureModeChanged(active,config);if(active)pipSession=true;nativePipMode(active);
        getWindow().getDecorView().requestApplyInsets();
    }
    public void keepRoomAlive(boolean active){runOnUiThread(()->{
        if(roomActive==active)return;
        roomActive=active;
        Intent service=new Intent(this,RoomPresenceService.class);
        try {
            if(active){startForegroundService(service);
                if(Build.VERSION.SDK_INT>=33&&!notificationAsked&&checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)!=android.content.pm.PackageManager.PERMISSION_GRANTED){notificationAsked=true;requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS},77);}
            }else stopService(service);
        }catch(RuntimeException error){roomActive=false;DebugLog.record("room_presence_service_error="+error.getClass().getSimpleName());}
    });}
    public void editRoom(String current){runOnUiThread(()->{
        android.widget.EditText name=new android.widget.EditText(this);name.setSingleLine(true);name.setHint(L10n.t("Room name"));name.setText(current);name.setFilters(new android.text.InputFilter[]{new android.text.InputFilter.LengthFilter(64)});
        new android.app.AlertDialog.Builder(this).setTitle(L10n.t("Room name")).setView(name).setPositiveButton(L10n.t("Save"),(d,w)->nativeRoomEdited(name.getText().toString())).setNegativeButton(L10n.t("Cancel"),null).show();
    });}
    public void editConfig(String current){runOnUiThread(()->{
        android.widget.EditText text=new android.widget.EditText(this);text.setText(current);text.setTypeface(android.graphics.Typeface.MONOSPACE);text.setTextSize(13);text.setInputType(android.text.InputType.TYPE_CLASS_TEXT|android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE|android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        android.widget.ScrollView scroll=new android.widget.ScrollView(this);scroll.addView(text);
        android.app.AlertDialog dialog=new android.app.AlertDialog.Builder(this).setTitle(L10n.t("config.toml — restart to apply")).setView(scroll).setPositiveButton(L10n.t("Save"),null).setNegativeButton(L10n.t("Cancel"),null).create();
        dialog.setOnShowListener(d->dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE).setOnClickListener(v->{String error=nativeSaveConfig(text.getText().toString());if(error==null){android.widget.Toast.makeText(this,L10n.t("Saved. Restart the app to apply settings."),android.widget.Toast.LENGTH_LONG).show();dialog.dismiss();}else new android.app.AlertDialog.Builder(this).setMessage(error).setPositiveButton("OK",null).show();}));dialog.show();
    });}
    private boolean handlesViewerBack;
    private boolean immersive;
    private android.window.OnBackInvokedCallback backCallback;
    public static native void nativeBack();
    public static native void nativeVideoReady();
    public static native void nativeDebugLog(String text);
    public void setDebugLogging(boolean enabled) { DebugLog.enabled=enabled;if(enabled) DebugLog.record("phone="+android.os.Build.MANUFACTURER+" "+android.os.Build.MODEL+" api="+Build.VERSION.SDK_INT+" refresh_hz="+getWindowManager().getDefaultDisplay().getRefreshRate()); }
    public static native void nativeInsets(int left,int top,int right,int bottom);
    static { System.loadLibrary("p2pshare_android"); }
    public static native void nativeNameEdited(String name);
    public void setUiLanguage(String language) { L10n.set(this,language); }
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        L10n.init(this);
        setVolumeControlStream(android.media.AudioManager.STREAM_MUSIC);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if(Build.VERSION.SDK_INT>=30) getWindow().setDecorFitsSystemWindows(false);
        else getWindow().getDecorView().setSystemUiVisibility(android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE | android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        if(Build.VERSION.SDK_INT>=28) {
            WindowManager.LayoutParams attributes=getWindow().getAttributes();
            attributes.layoutInDisplayCutoutMode=WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            getWindow().setAttributes(attributes);
        }
        getWindow().getDecorView().setOnApplyWindowInsetsListener((view,insets) -> {
            int left,top,right,bottom;
            if(Build.VERSION.SDK_INT>=30) {
                android.graphics.Insets safe=insets.getInsets(android.view.WindowInsets.Type.systemBars() | android.view.WindowInsets.Type.displayCutout());
                left=safe.left;top=safe.top;right=safe.right;bottom=safe.bottom;
            } else {
                left=insets.getSystemWindowInsetLeft();top=insets.getSystemWindowInsetTop();right=insets.getSystemWindowInsetRight();bottom=insets.getSystemWindowInsetBottom();
                if(Build.VERSION.SDK_INT>=28 && insets.getDisplayCutout()!=null) {
                    android.view.DisplayCutout c=insets.getDisplayCutout();left=Math.max(left,c.getSafeInsetLeft());top=Math.max(top,c.getSafeInsetTop());right=Math.max(right,c.getSafeInsetRight());bottom=Math.max(bottom,c.getSafeInsetBottom());
                }
            }
            nativeInsets(left,top,right,bottom);return insets;
        });
        getWindow().getDecorView().requestApplyInsets();
    }
    /** Presentation only. Never changes the room or capture service. */
    public void setViewerMode(boolean fullscreen,boolean handlesBack) {
        runOnUiThread(() -> {
            handlesViewerBack=handlesBack;
            if(Build.VERSION.SDK_INT>=33) {
                if(handlesBack && backCallback==null) {
                    backCallback=() -> nativeBack();
                    getOnBackInvokedDispatcher().registerOnBackInvokedCallback(android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT,backCallback);
                } else if(!handlesBack && backCallback!=null) {
                    getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backCallback);backCallback=null;
                }
            }
            if(immersive!=fullscreen) {
                immersive=fullscreen;
                if(Build.VERSION.SDK_INT>=30) {
                    android.view.WindowInsetsController controller=getWindow().getInsetsController();
                    if(controller!=null) {
                        controller.setSystemBarsBehavior(android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                        if(fullscreen) controller.hide(android.view.WindowInsets.Type.systemBars()); else controller.show(android.view.WindowInsets.Type.systemBars());
                    }
                } else {
                    getWindow().getDecorView().setSystemUiVisibility(fullscreen ? android.view.View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY | android.view.View.SYSTEM_UI_FLAG_FULLSCREEN | android.view.View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE | android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION : android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE | android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
                }
                getWindow().getDecorView().requestApplyInsets();
            }
        });
    }
    @Override public boolean dispatchKeyEvent(android.view.KeyEvent event) {
        if(Build.VERSION.SDK_INT<33 && handlesViewerBack && event.getKeyCode()==android.view.KeyEvent.KEYCODE_BACK) {
            if(event.getAction()==android.view.KeyEvent.ACTION_UP && !event.isCanceled()) nativeBack();
            return true;
        }
        return super.dispatchKeyEvent(event);
    }
    @Override public void onBackPressed() {if(Build.VERSION.SDK_INT<33 && handlesViewerBack) nativeBack(); else super.onBackPressed();}
    public void copyInvite(String text) { runOnUiThread(() -> ((ClipboardManager)getSystemService(CLIPBOARD_SERVICE)).setPrimaryClip(ClipData.newPlainText(L10n.t("P2PShare invitation"), text))); }
    public String pasteInvite() {
        ClipboardManager clipboard = (ClipboardManager)getSystemService(CLIPBOARD_SERVICE);
        ClipData clip = clipboard.getPrimaryClip();
        return clip != null && clip.getItemCount() > 0 ? clip.getItemAt(0).coerceToText(this).toString() : "";
    }
    public void editName(String current) {
        runOnUiThread(() -> {
            android.widget.EditText text = new android.widget.EditText(this);
            text.setSingleLine(true); text.setText(current); text.setSelection(text.length());
            text.setFilters(new android.text.InputFilter[]{new android.text.InputFilter.LengthFilter(32)});
            android.app.AlertDialog dialog = new android.app.AlertDialog.Builder(this).setTitle(L10n.t("Display name")).setView(text).setPositiveButton(L10n.t("Save"), (d, which) -> nativeNameEdited(text.getText().toString())).setNegativeButton(L10n.t("Cancel"), null).create();
            dialog.setOnShowListener(d -> { text.requestFocus(); dialog.getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE); });
            dialog.show();
        });
    }
    public boolean createDecoder(String peer, int texture, int width, int height, String mime) { return VideoDecoder.create(peer, texture, width, height, mime); }
    public boolean feedDecoder(String peer, byte[] bytes, long micros, boolean keyframe,long ageUs,String mime) { return VideoDecoder.feed(peer, bytes, micros, keyframe,ageUs,mime); }
    public boolean updateDecoder(String peer, float[] matrix) { return VideoDecoder.update(peer, matrix); }
    public String decoderDiagnostics() { return VideoDecoder.diagnostics(); }
    public void removeDecoder(String peer) { VideoDecoder.remove(peer); }
    @Override public void onDestroy() {
        if(Build.VERSION.SDK_INT>=33 && backCallback!=null) {getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backCallback);backCallback=null;}
        super.onDestroy();
    }
}





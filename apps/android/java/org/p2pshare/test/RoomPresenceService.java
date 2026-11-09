package org.p2pshare.test;

import android.app.*;
import android.content.*;
import android.content.pm.ServiceInfo;
import android.os.*;

/** Keeps only the explicitly joined room alive; never captures a screen. */
public final class RoomPresenceService extends Service {
    private static final String CHANNEL="room_presence";
    private static final int ID=7;
    private PowerManager.WakeLock wakeLock;
    @Override public int onStartCommand(Intent intent,int flags,int startId) {
        L10n.init(this);
        if(intent!=null&&"leave".equals(intent.getAction())) {
            P2PShareActivity.nativeLeaveRoom();stopForeground(STOP_FOREGROUND_REMOVE);stopSelf();return START_NOT_STICKY;
        }
        NotificationManager manager=(NotificationManager)getSystemService(NOTIFICATION_SERVICE);
        manager.createNotificationChannel(new NotificationChannel(CHANNEL,L10n.t("Active room"),NotificationManager.IMPORTANCE_LOW));
        Intent open=new Intent(this,P2PShareActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent view=PendingIntent.getActivity(this,0,open,PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        PendingIntent leave=PendingIntent.getService(this,1,new Intent(this,RoomPresenceService.class).setAction("leave"),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        Notification notification=new Notification.Builder(this,CHANNEL).setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(L10n.t("P2PShare room active")).setContentText(L10n.t("Staying connected while the viewer is in the background"))
            .setContentIntent(view).setOngoing(true).setCategory(Notification.CATEGORY_SERVICE)
            .addAction(new Notification.Action.Builder(null,L10n.t("Leave room"),leave).build()).build();
        if(Build.VERSION.SDK_INT>=34)startForeground(ID,notification,ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        else startForeground(ID,notification);
        if(wakeLock==null){wakeLock=((PowerManager)getSystemService(POWER_SERVICE)).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"P2PShare:RoomPresence");wakeLock.setReferenceCounted(false);wakeLock.acquire();}
        DebugLog.record("room_presence_service_started");
        return START_NOT_STICKY;
    }
    @Override public void onTaskRemoved(Intent rootIntent){P2PShareActivity.nativeLeaveRoom();stopSelf();}
    @Override public void onDestroy(){if(wakeLock!=null&&wakeLock.isHeld())wakeLock.release();DebugLog.record("room_presence_service_stopped");super.onDestroy();}
    @Override public IBinder onBind(Intent intent){return null;}
}

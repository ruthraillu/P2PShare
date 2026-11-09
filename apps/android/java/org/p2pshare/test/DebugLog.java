package org.p2pshare.test;
final class DebugLog {
 static volatile boolean enabled;
 static void record(String message) { if(enabled) P2PShareActivity.nativeDebugLog("android t_ms="+(System.nanoTime()/1000000)+" "+message+"\n"); }
}

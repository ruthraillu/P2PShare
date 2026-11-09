package org.p2pshare.test;
import java.util.ArrayDeque;
/** Compressed frames must stay ordered: only decoded display frames may be coalesced. */
final class DecoderQueue {
    static final int MAX_FRAMES=24, MAX_BYTES=8*1024*1024, MAX_FRAME_BYTES=2*1024*1024;
    static final class Input {
        final byte[] data; final long micros,receivedNs;
        Input(byte[] data,long micros,long ageUs) { this.data=data;this.micros=micros;this.receivedNs=System.nanoTime()-ageUs*1000; }
    }
    private final ArrayDeque<Input> frames=new ArrayDeque<>();
    private volatile int bytes;
    volatile int queued;
    volatile long dropped,overflows;
    private volatile boolean waiting=true;
    boolean offer(byte[] data,long micros,boolean keyframe) { return offer(data,micros,keyframe,0); }
    boolean offer(byte[] data,long micros,boolean keyframe,long ageUs) {
        if(data.length>MAX_FRAME_BYTES) { lost();return false; }
        if(waiting && !keyframe) { dropped++;return true; }
        if(frames.size()>=MAX_FRAMES || bytes+data.length>MAX_BYTES) {
            overflows++;dropped+=frames.size();lost(); if(!keyframe) { dropped++;return false; }
        }
        if(keyframe) waiting=false;
        frames.addLast(new Input(data,micros,ageUs));bytes+=data.length;queued=frames.size();return true;
    }
    Input peek() { return frames.peekFirst(); }
    void remove() { Input head=frames.removeFirst();bytes-=head.data.length;queued=frames.size(); }
    void lost() { frames.clear();bytes=0;queued=0;waiting=true; }
    boolean waiting() { return waiting; }
    boolean isEmpty() { return frames.isEmpty(); }
}

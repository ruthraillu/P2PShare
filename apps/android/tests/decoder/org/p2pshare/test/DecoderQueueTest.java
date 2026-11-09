package org.p2pshare.test;
public final class DecoderQueueTest {
    public static void main(String[] args) {
        DecoderQueue q=new DecoderQueue();byte[] key={1},p={2};
        assert q.offer(p,0,false) && q.isEmpty();
        assert q.offer(key,1,true);assert q.offer(p,2,false);assert q.offer(p,3,false);
        assert q.peek().micros==1;q.remove();assert q.peek().micros==2;q.remove();assert q.peek().micros==3;q.remove();assert q.isEmpty();
        // A brief recovery/layout pause must preserve a burst exceeding the old 8-frame limit.
        for(int i=0;i<20;i++) assert q.offer(p,20+i,false);
        for(int i=0;i<20;i++) { assert q.peek().micros==20+i;q.remove(); }
        for(int i=0;i<DecoderQueue.MAX_FRAMES;i++) assert q.offer(p,i,false);
        assert !q.offer(p,9,false);assert q.isEmpty();assert q.offer(p,10,false) && q.isEmpty();
        assert q.offer(key,11,true) && !q.isEmpty();q.remove();
        assert !q.offer(new byte[DecoderQueue.MAX_FRAME_BYTES+1],12,true);assert q.isEmpty();
        assert q.offer(new byte[DecoderQueue.MAX_FRAME_BYTES],13,true);
        for(int i=0;i<3;i++) assert q.offer(new byte[DecoderQueue.MAX_FRAME_BYTES],14+i,false);
        assert !q.offer(p,18,false);assert q.isEmpty();
        System.out.println("Decoder queue ordering, backpressure retention, overflow recovery and byte bounds passed");
    }
}

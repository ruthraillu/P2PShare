package org.p2pshare.test;
import android.app.Activity;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.net.Uri;
import android.widget.Toast;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
/** Picker URI is read once in a worker, downsampled before allocation; no storage permission. */
final class ProfileImages {
 static void load(Activity activity,Uri uri){new Thread(()->{
  Bitmap bitmap=null;
  try{
   BitmapFactory.Options o=new BitmapFactory.Options();o.inJustDecodeBounds=true;
   try(InputStream input=activity.getContentResolver().openInputStream(uri)){BitmapFactory.decodeStream(input,null,o);}
   if(o.outWidth<=0||o.outHeight<=0||o.outWidth>32768||o.outHeight>32768)throw new Exception("Unsupported image dimensions");
   int sample=1;while(Math.max(o.outWidth,o.outHeight)/sample>1024)sample*=2;
   o=new BitmapFactory.Options();o.inSampleSize=sample;
   try(InputStream input=activity.getContentResolver().openInputStream(uri)){bitmap=BitmapFactory.decodeStream(input,null,o);}
   if(bitmap==null)throw new Exception("Cannot decode image");
   int orientation=1;
   try(InputStream input=activity.getContentResolver().openInputStream(uri)){orientation=new android.media.ExifInterface(input).getAttributeInt(android.media.ExifInterface.TAG_ORIENTATION,1);}catch(Exception ignored){}
   android.graphics.Matrix transform=new android.graphics.Matrix();
   switch(orientation){case 2:transform.setScale(-1,1);break;case 3:transform.setRotate(180);break;case 4:transform.setScale(1,-1);break;case 5:transform.setRotate(90);transform.postScale(-1,1);break;case 6:transform.setRotate(90);break;case 7:transform.setRotate(-90);transform.postScale(-1,1);break;case 8:transform.setRotate(-90);break;}
   if(!transform.isIdentity()){Bitmap oriented=Bitmap.createBitmap(bitmap,0,0,bitmap.getWidth(),bitmap.getHeight(),transform,true);if(oriented!=bitmap){bitmap.recycle();bitmap=oriented;}}
   ByteArrayOutputStream bytes=new ByteArrayOutputStream();if(!bitmap.compress(Bitmap.CompressFormat.PNG,100,bytes))throw new Exception("Cannot read image");
   if(bytes.size()>16*1024*1024)throw new Exception("Image too large");
   P2PShareActivity.nativeAvatarPicked(bytes.toByteArray());
  }catch(Exception|OutOfMemoryError e){DebugLog.record("avatar_picker_error="+e);activity.runOnUiThread(()->Toast.makeText(activity,L10n.t("Unable to open this image"),Toast.LENGTH_LONG).show());}
  finally{if(bitmap!=null)bitmap.recycle();}
 },"avatar-picker").start();}
}

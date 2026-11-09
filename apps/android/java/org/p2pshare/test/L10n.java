package org.p2pshare.test;
import android.content.Context;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
/** Shared catalog; never translates user names, app labels or protocol values. */
final class L10n {
 private static volatile String language="en";
 private static volatile JSONObject catalog=new JSONObject();
 private static boolean loaded;
 static synchronized void init(Context context){
  language=context.getSharedPreferences("interface",0).getString("language","en");
  if(loaded)return;
  try(InputStream in=context.getAssets().open("i18n/pt-BR.json");ByteArrayOutputStream out=new ByteArrayOutputStream()){
   byte[] buf=new byte[4096];int n;while((n=in.read(buf))!=-1)out.write(buf,0,n);
   catalog=new JSONObject(out.toString("UTF-8"));loaded=true;
  }catch(Exception ignored){}
 }
 static void set(Context context,String value){init(context);language=value;context.getSharedPreferences("interface",0).edit().putString("language",value).apply();}
 static String t(String value){return "pt-BR".equals(language)?catalog.optString(value,value):value;}
}

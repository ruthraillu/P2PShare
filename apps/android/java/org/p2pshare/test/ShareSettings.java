package org.p2pshare.test;
import android.app.*;
import android.content.*;
import android.content.pm.*;
import android.os.Build;
import android.widget.*;
import java.util.*;
/** Android sender preferences, separate from capture and playback. */
final class ShareSettings {
 static void show(P2PShareActivity a){
  android.content.SharedPreferences p=a.getSharedPreferences("sharing",0);
  LinearLayout box=new LinearLayout(a);box.setOrientation(LinearLayout.VERTICAL);box.setPadding(24,8,24,8);
  Spinner codec=spinner(a,box,L10n.t("Codec"),new String[]{"AVC / H.264","HEVC / H.265"},p.getInt("codec",0));
  Spinner res=spinner(a,box,L10n.t("Resolution (aspect preserved)"),new String[]{"480p","720p","1080p","1440p","2160p"},p.getInt("res",1));
  Spinner fps=spinner(a,box,L10n.t("Frame rate"),new String[]{"30","60","90","120"},p.getInt("fps",0));
  EditText bitrate=number(a,box,L10n.t("Video bitrate (kbps, 100–50000)"),p.getInt("bitrate",6000));
  CheckBox audio=new CheckBox(a);audio.setText(L10n.t("Share playback audio (Android 10+) — no microphone"));audio.setChecked(p.getBoolean("audio",true));audio.setEnabled(Build.VERSION.SDK_INT>=29);box.addView(audio);
  PackageManager pm=a.getPackageManager();
  Intent launcher=new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
  List<ResolveInfo> rawApps=pm.queryIntentActivities(launcher,0);
  class AppEntry {
   final String label;
   final String pkg;
   final int uid;
   AppEntry(String label,String pkg,int uid){this.label=label;this.pkg=pkg;this.uid=uid;}
  }
  ArrayList<AppEntry> entries=new ArrayList<>(rawApps.size());
  HashSet<Integer> seen=new HashSet<>();
  int ownUid=a.getApplicationInfo().uid;
  for(ResolveInfo app:rawApps){
   if(app.activityInfo==null||app.activityInfo.applicationInfo==null)continue;
   int uid=app.activityInfo.applicationInfo.uid;
   if(uid==ownUid||!seen.add(uid))continue;
   entries.add(new AppEntry(app.loadLabel(pm).toString(),app.activityInfo.applicationInfo.packageName,uid));
  }
  entries.sort((x,y)->x.label.compareToIgnoreCase(y.label));
  ArrayList<String> names=new ArrayList<>(entries.size()+1);ArrayList<Integer> uids=new ArrayList<>(entries.size()+1);
  HashMap<Integer,String> uidToPkg=new HashMap<>();
  names.add(L10n.t("All media / game audio (even with single-app video)"));uids.add(-1);
  for(AppEntry entry:entries){names.add(entry.label);uids.add(entry.uid);uidToPkg.put(entry.uid,entry.pkg);}
  int selectedUid=-1;
  String savedPkg=p.getString("audio_pkg",null);
  if(savedPkg!=null){
   if(!savedPkg.isEmpty()){try{selectedUid=pm.getPackageUid(savedPkg,0);}catch(Exception ignored){selectedUid=-1;}}
  }else{
   selectedUid=p.getInt("uid",-1);
  }
  AudioSourcePicker source=new AudioSourcePicker(a,box,names,uids,selectedUid);
  EditText opus=number(a,box,L10n.t("Opus audio bitrate (kbps, 64–512)"),p.getInt("opus",192));
  TextView note=new TextView(a);note.setText(L10n.t("Some apps prohibit audio capture. Video and audio sources are independent. Encoder support and attainable FPS depend on your phone. Continue to select a new screen or app; connected viewers keep watching."));box.addView(note);
  ScrollView scroll=new ScrollView(a);scroll.addView(box);
  AlertDialog dialog=new AlertDialog.Builder(a).setTitle(L10n.t("Share screen settings")).setView(scroll).setPositiveButton(L10n.t("Continue"),null).setNegativeButton(L10n.t("Cancel"),null).create();
  dialog.setOnShowListener(d->dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v->{try{
    int b=Integer.parseInt(bitrate.getText().toString()),o=Integer.parseInt(opus.getText().toString());if(b<100||b>50000||o<64||o>512)throw new IllegalArgumentException();
    int chosenUid=source.uid();String chosenPkg=chosenUid>=0?uidToPkg.get(chosenUid):null;if(chosenPkg==null)chosenPkg="";
    p.edit().putInt("codec",codec.getSelectedItemPosition()).putInt("res",res.getSelectedItemPosition()).putInt("fps",fps.getSelectedItemPosition()).putInt("bitrate",b).putInt("opus",o).putInt("uid",chosenUid).putString("audio_pkg",chosenPkg).putBoolean("audio",audio.isChecked()&&Build.VERSION.SDK_INT>=29).apply();dialog.dismiss();a.requestShareConsent();
   }catch(Exception e){Toast.makeText(a,L10n.t("Check bitrate ranges"),Toast.LENGTH_LONG).show();}}));dialog.show();
 }
 static Spinner spinner(Activity a,LinearLayout box,String label,String[] values,int selected){TextView t=new TextView(a);t.setText(label);box.addView(t);Spinner s=new Spinner(a);s.setAdapter(new ArrayAdapter<String>(a,android.R.layout.simple_spinner_dropdown_item,values));s.setSelection(Math.min(Math.max(0,selected),values.length-1));box.addView(s);return s;}
 static EditText number(Activity a,LinearLayout box,String label,int value){TextView t=new TextView(a);t.setText(label);box.addView(t);EditText e=new EditText(a);e.setSingleLine(true);e.setInputType(android.text.InputType.TYPE_CLASS_NUMBER);e.setText(Integer.toString(value));box.addView(e);return e;}
}

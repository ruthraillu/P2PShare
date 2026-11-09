package org.p2pshare.test;
import android.app.Activity;
import android.app.AlertDialog;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.WindowManager;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
/** Search changes only the visible list. The selected audio UID remains stable. */
final class AudioSourcePicker {
 private final Activity activity;
 private final ArrayList<String> names;
 private final ArrayList<Integer> uids;
 private int selected;
 final Button button;
 AudioSourcePicker(Activity activity,LinearLayout parent,List<String> names,List<Integer> uids,int selectedUid){
  this.activity=activity;this.names=new ArrayList<>(names);this.uids=new ArrayList<>(uids);
  selected=this.uids.contains(selectedUid)?selectedUid:-1;
  TextView label=new TextView(activity);label.setText(L10n.t("Audio source — choose the same app for app-only audio"));parent.addView(label);
  button=new Button(activity);button.setAllCaps(false);updateLabel();button.setOnClickListener(v->show());
  parent.addView(button,new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT,LinearLayout.LayoutParams.WRAP_CONTENT));
 }
 int uid(){return selected;}
 private void updateLabel(){int index=uids.indexOf(selected);button.setText(index>=0?names.get(index):names.get(0));}
 private void show(){
  LinearLayout header=new LinearLayout(activity);header.setOrientation(LinearLayout.VERTICAL);
  int pad=Math.round(20*activity.getResources().getDisplayMetrics().density);header.setPadding(pad,pad,pad,0);
  TextView title=new TextView(activity);title.setText(L10n.t("Audio source"));title.setTextSize(20);header.addView(title);
  EditText search=new EditText(activity);search.setSingleLine(true);search.setHint(L10n.t("Search apps"));
  search.setInputType(android.text.InputType.TYPE_CLASS_TEXT|android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
  header.addView(search,new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT,LinearLayout.LayoutParams.WRAP_CONTENT));
  final ArrayList<Integer> visible=new ArrayList<>();for(int i=0;i<names.size();i++)visible.add(i);
  ArrayAdapter<String> adapter=new ArrayAdapter<String>(activity,android.R.layout.simple_list_item_single_choice,new ArrayList<>(names)){
   @Override public boolean isEnabled(int position){return !visible.isEmpty();}
  };
  AlertDialog dialog=new AlertDialog.Builder(activity).setCustomTitle(header).setSingleChoiceItems(adapter,uids.indexOf(selected),(d,position)->{
   if(position<0||position>=visible.size())return;
   selected=uids.get(visible.get(position));updateLabel();d.dismiss();
  }).setNegativeButton(L10n.t("Cancel"),null).create();
  search.addTextChangedListener(new TextWatcher(){
   public void beforeTextChanged(CharSequence s,int start,int count,int after){}
   public void onTextChanged(CharSequence s,int start,int before,int count){
    String query=s.toString().trim().toLowerCase(Locale.ROOT);visible.clear();ArrayList<String> labels=new ArrayList<>();int checked=-1;
    for(int i=0;i<names.size();i++){if(names.get(i).toLowerCase(Locale.ROOT).contains(query)){if(uids.get(i)==selected)checked=visible.size();visible.add(i);labels.add(names.get(i));}}
    if(labels.isEmpty())labels.add(L10n.t("No matching apps"));
    adapter.clear();adapter.addAll(labels);adapter.notifyDataSetChanged();
    if(dialog.getListView()!=null){dialog.getListView().clearChoices();if(checked>=0)dialog.getListView().setItemChecked(checked,true);dialog.getListView().setSelection(0);}
   }
   public void afterTextChanged(Editable s){}
  });
  dialog.show();
  if(dialog.getWindow()!=null)dialog.getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE|WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN);
 }
}

//! Shared UI localization. User names, window titles, protocol values and logs stay untouched.
use std::{borrow::Cow,cell::Cell,collections::HashMap,sync::OnceLock};
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
pub enum Language {
 #[default]
 #[serde(rename="en",alias="en-US",alias="en-us")] English,
 #[serde(rename="pt-BR",alias="pt-br")] PtBr,
}
impl Language {pub fn code(self)->&'static str{match self{Self::English=>"en",Self::PtBr=>"pt-BR"}}}
thread_local!{static CURRENT:Cell<Language>=const{Cell::new(Language::English)};}
pub fn current()->Language{CURRENT.with(Cell::get)}
pub struct Scope(Language);
pub fn scope(language:Language)->Scope{Scope(CURRENT.with(|c|c.replace(language)))}
impl Drop for Scope{fn drop(&mut self){CURRENT.with(|c|c.set(self.0));}}
fn catalog()->&'static HashMap<String,String>{static PT:OnceLock<HashMap<String,String>>=OnceLock::new();PT.get_or_init(||serde_json::from_str(include_str!("i18n/pt_BR.json")).expect("Validated Portuguese UI catalog"))}
pub fn text(key:&'static str)->&'static str{if current()==Language::English{return key}catalog().get(key).map(String::as_str).unwrap_or(key)}
pub fn status(value:&str)->Cow<'_,str>{
 if current()==Language::English{return Cow::Borrowed(value)}
 if let Some(text)=catalog().get(value){return Cow::Borrowed(text);}
 if value.starts_with("Direct IPv6: "){return Cow::Owned(value.replace("Direct IPv6: ","IPv6 direto: ").replace("Direct IPv4: ","IPv4 direto: ").replace("Cloudflare signaling","Sinalização Cloudflare"));}
 for (from,to) in [("Screen sharing failed: ","Falha na transmissão: "),("Could not save language: ","Não foi possível salvar o idioma: "),("Could not apply settings: ","Não foi possível aplicar os ajustes: "),("Could not list windows: ","Não foi possível listar as janelas: "),("Settings not saved: ","Ajustes não salvos: "),("Starting ","Iniciando "),("Could not save: ","Não foi possível salvar: ")]{if let Some(rest)=value.strip_prefix(from){return Cow::Owned(format!("{to}{rest}"));}}
 Cow::Borrowed(value)
}
#[macro_export]macro_rules! tr{($key:literal)=>{$crate::i18n::text($key)};}
include!("i18n_formats.rs");
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn scopes_are_per_thread_and_restore(){assert_eq!(current(),Language::English);{let _guard=scope(Language::PtBr);assert_eq!(text("Settings"),"Ajustes");assert_eq!(std::thread::spawn(||text("Settings")).join().unwrap(),"Settings");}assert_eq!(text("Settings"),"Settings");}
 #[test]fn known_formats_keep_user_values_unchanged(){let _guard=scope(Language::PtBr);assert_eq!(trf!("Watching {}","English Username"),"Assistindo English Username");assert_eq!(text("Unlisted source title"),"Unlisted source title");}
 #[test]fn config_codes_round_trip(){assert_eq!(serde_json::to_string(&Language::PtBr).unwrap(),"\"pt-BR\"");assert_eq!(serde_json::from_str::<Language>("\"pt-br\"").unwrap(),Language::PtBr);}
}

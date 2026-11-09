// Generated from i18n/pt_BR.json; format identifiers remain caller arguments.
#[macro_export]
macro_rules! trf {
("USERS  /  {}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("USUÁRIOS  /  {}" $(,$($args)*)?)} else {format!("USERS  /  {}" $(,$($args)*)?)} }};
("Users · {}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Usuários · {}" $(,$($args)*)?)} else {format!("Users · {}" $(,$($args)*)?)} }};
("{} members" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("{} participantes" $(,$($args)*)?)} else {format!("{} members" $(,$($args)*)?)} }};
("{} · {} users" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("{} · {} usuários" $(,$($args)*)?)} else {format!("{} · {} users" $(,$($args)*)?)} }};
("Watching {}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Assistindo {}" $(,$($args)*)?)} else {format!("Watching {}" $(,$($args)*)?)} }};
("Watching {}  ›" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Assistindo {}  ›" $(,$($args)*)?)} else {format!("Watching {}  ›" $(,$($args)*)?)} }};
("Watching {} screens" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Assistindo {} telas" $(,$($args)*)?)} else {format!("Watching {} screens" $(,$($args)*)?)} }};
("Previous names   {}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Nomes anteriores   {}" $(,$($args)*)?)} else {format!("Previous names   {}" $(,$($args)*)?)} }};
("Previous avatars   {}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Fotos anteriores   {}" $(,$($args)*)?)} else {format!("Previous avatars   {}" $(,$($args)*)?)} }};
("Show all {} names" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Ver os {} nomes" $(,$($args)*)?)} else {format!("Show all {} names" $(,$($args)*)?)} }};
("Connected to {}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Conectado a {}" $(,$($args)*)?)} else {format!("Connected to {}" $(,$($args)*)?)} }};
("Could not save: {e}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Não foi possível salvar: {e}" $(,$($args)*)?)} else {format!("Could not save: {e}" $(,$($args)*)?)} }};
("Text contrast  {contrast:.1}:1{}" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Contraste do texto  {contrast:.1}:1{}" $(,$($args)*)?)} else {format!("Text contrast  {contrast:.1}:1{}" $(,$($args)*)?)} }};
("Screen volume. Mouse wheel: {}% per tick." $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Volume da tela. Roda do mouse: {}% por passo." $(,$($args)*)?)} else {format!("Screen volume. Mouse wheel: {}% per tick." $(,$($args)*)?)} }};
("{stats}\nBitrate is the sharer’s configured video target." $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("{stats}\nA taxa de bits é o valor configurado por quem transmite." $(,$($args)*)?)} else {format!("{stats}\nBitrate is the sharer’s configured video target." $(,$($args)*)?)} }};
("{}\nClick or tap to focus this screen" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("{}\nClique ou toque para destacar esta tela" $(,$($args)*)?)} else {format!("{}\nClick or tap to focus this screen" $(,$($args)*)?)} }};
("{}\nClick to change room" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("{}\nClique para alterar a sala" $(,$($args)*)?)} else {format!("{}\nClick to change room" $(,$($args)*)?)} }};
("P2PShare {} • viewer diagnostics" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("P2PShare {} • diagnóstico de reprodução" $(,$($args)*)?)} else {format!("P2PShare {} • viewer diagnostics" $(,$($args)*)?)} }};
("Local capture: {:.1} FPS" $(, $($args:tt)*)?) => {{ if $crate::i18n::current()==$crate::i18n::Language::PtBr {format!("Captura local: {:.1} FPS" $(,$($args)*)?)} else {format!("Local capture: {:.1} FPS" $(,$($args)*)?)} }};
($($args:tt)*) => {format!($($args)*)};
}

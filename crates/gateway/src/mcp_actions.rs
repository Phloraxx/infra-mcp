//! Small action enums keep both deserialization and exported JSON Schema honest.
use schemars::JsonSchema;
use serde::Deserialize;
macro_rules! actions {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Debug, Deserialize, JsonSchema)]
        pub(crate) enum $name { $(#[serde(rename = $wire)] $variant),+ }
        impl $name {
            pub(crate) fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }
    };
}
actions!(FileActionName { Read=>"read", Write=>"write", Append=>"append", ExactEdit=>"exact_edit", Hash=>"hash", List=>"list", Stat=>"stat", Mkdir=>"mkdir", Copy=>"copy", Move=>"move", Trash=>"trash", DeletePermanent=>"delete_permanent" });
actions!(SearchModeName { Files=>"files", Content=>"content" });
actions!(ProcessActionName { List=>"list", Terminate=>"terminate" });
actions!(JobActionName { Start=>"start", List=>"list", Read=>"read", Cancel=>"cancel" });
actions!(TerminalActionName { Open=>"open", List=>"list", Read=>"read", Write=>"write", Resize=>"resize", Close=>"close" });
actions!(TransferActionName { UploadBegin=>"upload_begin", UploadChunk=>"upload_chunk", UploadCommit=>"upload_commit", DownloadBegin=>"download_begin", DownloadChunk=>"download_chunk", Status=>"status", List=>"list", Cancel=>"cancel" });
actions!(DesktopActionName { Permissions=>"permissions", Displays=>"displays", Windows=>"windows", Screenshot=>"screenshot", Focus=>"focus", Click=>"click", TypeText=>"type_text", Key=>"key", ClipboardGet=>"clipboard_get", ClipboardSet=>"clipboard_set" });
actions!(SystemActionName { Summary=>"summary", Disks=>"disks", Networks=>"networks" });
actions!(NetworkActionName { Listeners=>"listeners", Resolve=>"resolve", TcpCheck=>"tcp_check" });
actions!(ServiceActionName { List=>"list", Status=>"status", Start=>"start", Stop=>"stop", Restart=>"restart" });
actions!(ContainerActionName { List=>"list", Inspect=>"inspect", Logs=>"logs", Start=>"start", Stop=>"stop", Restart=>"restart" });
actions!(GitActionName { Status=>"status", Diff=>"diff", Log=>"log", Branches=>"branches", Remotes=>"remotes", Fetch=>"fetch", Pull=>"pull", Push=>"push" });

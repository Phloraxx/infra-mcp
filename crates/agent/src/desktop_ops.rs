use infra_protocol::{DesktopAction, DesktopRequest, OperationResult};
use serde_json::Value;

#[cfg(any(target_os = "macos", windows))]
use base64::{Engine as _, engine::general_purpose::STANDARD};
#[cfg(any(target_os = "macos", windows))]
use serde_json::json;
#[cfg(any(target_os = "macos", windows))]
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
#[cfg(any(target_os = "macos", windows))]
use uuid::Uuid;

#[cfg(any(target_os = "macos", windows))]
const MAX_TEXT_OUTPUT: usize = 4 * 1024 * 1024;
#[cfg(any(target_os = "macos", windows))]
const MAX_SCREENSHOT_BYTES: usize = 5 * 1024 * 1024;
#[cfg(any(target_os = "macos", windows))]
const MAX_INPUT_TEXT: usize = 1024 * 1024;

pub fn handle(request: DesktopRequest) -> OperationResult {
    match handle_inner(request.action) {
        Ok(body) => OperationResult::Ok { body },
        Err(error) => OperationResult::Failed {
            error: error.to_string(),
        },
    }
}

#[cfg(target_os = "macos")]
fn handle_inner(action: DesktopAction) -> anyhow::Result<Value> {
    match action {
        DesktopAction::Displays => macos_displays(),
        DesktopAction::Windows => macos_windows(),
        DesktopAction::Screenshot {
            display,
            x,
            y,
            width,
            height,
        } => macos_screenshot(display, x, y, width, height),
        DesktopAction::Focus { app } => macos_focus(&app),
        DesktopAction::Click { x, y, button } => macos_click(x, y, button.as_deref()),
        DesktopAction::TypeText { text } => macos_type_text(&text),
        DesktopAction::Key { key, modifiers } => macos_key(&key, &modifiers),
        DesktopAction::ClipboardGet => macos_clipboard_get(),
        DesktopAction::ClipboardSet { text } => macos_clipboard_set(&text),
    }
}

#[cfg(windows)]
fn handle_inner(action: DesktopAction) -> anyhow::Result<Value> {
    match action {
        DesktopAction::Displays => windows_displays(),
        DesktopAction::Windows => windows_windows(),
        DesktopAction::Screenshot {
            display,
            x,
            y,
            width,
            height,
        } => windows_screenshot(display, x, y, width, height),
        DesktopAction::Focus { app } => windows_focus(&app),
        DesktopAction::Click { x, y, button } => windows_click(x, y, button.as_deref()),
        DesktopAction::TypeText { text } => windows_type_text(&text),
        DesktopAction::Key { key, modifiers } => windows_key(&key, &modifiers),
        DesktopAction::ClipboardGet => windows_clipboard_get(),
        DesktopAction::ClipboardSet { text } => windows_clipboard_set(&text),
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn handle_inner(_action: DesktopAction) -> anyhow::Result<Value> {
    anyhow::bail!("desktop control is currently implemented for macOS and Windows")
}

#[cfg(any(target_os = "macos", windows))]
fn ensure_text_limit(text: &str) -> anyhow::Result<()> {
    if text.len() > MAX_INPUT_TEXT {
        anyhow::bail!("desktop text input exceeds {} bytes", MAX_INPUT_TEXT);
    }
    Ok(())
}

#[cfg(any(target_os = "macos", windows))]
fn checked_output(mut command: Command, label: &str) -> anyhow::Result<Output> {
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{label} failed: {}", stderr.trim());
    }
    if output.stdout.len() > MAX_TEXT_OUTPUT {
        anyhow::bail!("{label} output exceeds {} bytes", MAX_TEXT_OUTPUT);
    }
    Ok(output)
}

#[cfg(any(target_os = "macos", windows))]
fn parsed_json_output(command: Command, label: &str) -> anyhow::Result<Value> {
    let output = checked_output(command, label)?;
    serde_json::from_slice(&output.stdout)
        .map_err(|error| anyhow::anyhow!("decode {label} output: {error}"))
}

#[cfg(any(target_os = "macos", windows))]
fn screenshot_path() -> PathBuf {
    std::env::temp_dir().join(format!("infra-desktop-{}.jpg", Uuid::new_v4()))
}

#[cfg(any(target_os = "macos", windows))]
fn screenshot_body(path: &Path) -> anyhow::Result<Value> {
    let bytes = fs::read(path)?;
    let _ = fs::remove_file(path);
    if bytes.is_empty() {
        anyhow::bail!("screenshot command produced an empty image")
    }
    if bytes.len() > MAX_SCREENSHOT_BYTES {
        anyhow::bail!(
            "screenshot is {} bytes; limit is {} bytes; capture a smaller region",
            bytes.len(),
            MAX_SCREENSHOT_BYTES
        )
    }
    Ok(json!({
        "mime_type": "image/jpeg",
        "size": bytes.len(),
        "data_base64": STANDARD.encode(bytes),
    }))
}

#[cfg(any(target_os = "macos", windows, test))]
fn validate_region(
    x: Option<i32>,
    y: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
) -> anyhow::Result<Option<(i32, i32, u32, u32)>> {
    match (x, y, width, height) {
        (None, None, None, None) => Ok(None),
        (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
            Ok(Some((x, y, width, height)))
        }
        _ => anyhow::bail!("screenshot region requires x, y, width and height together"),
    }
}

#[cfg(target_os = "macos")]
fn macos_displays() -> anyhow::Result<Value> {
    let mut command = Command::new("system_profiler");
    command.args(["SPDisplaysDataType", "-json", "-detailLevel", "mini"]);
    let value = parsed_json_output(command, "display discovery")?;
    Ok(json!({"platform": "macos", "raw": value}))
}

#[cfg(target_os = "macos")]
fn macos_windows() -> anyhow::Result<Value> {
    const SCRIPT: &str = r#"
const se = Application('System Events');
const out = [];
for (const p of se.applicationProcesses()) {
  let background = true;
  try { background = p.backgroundOnly(); } catch (_) {}
  if (background) continue;
  let app = ''; let pid = null; let frontmost = false;
  try { app = p.name(); } catch (_) {}
  try { pid = p.unixId(); } catch (_) {}
  try { frontmost = p.frontmost(); } catch (_) {}
  let windows = [];
  try { windows = p.windows(); } catch (_) {}
  if (windows.length === 0) out.push({app, pid, frontmost, title: null});
  for (const w of windows) {
    let title = null, position = null, size = null;
    try { title = w.name(); } catch (_) {}
    try { position = w.position(); } catch (_) {}
    try { size = w.size(); } catch (_) {}
    out.push({app, pid, frontmost, title, position, size});
  }
}
JSON.stringify(out);
"#;
    let mut command = Command::new("osascript");
    command.args(["-l", "JavaScript", "-e", SCRIPT]);
    let value = parsed_json_output(command, "window discovery")?;
    Ok(json!({"platform": "macos", "windows": value}))
}

#[cfg(target_os = "macos")]
fn macos_screenshot(
    display: Option<u32>,
    x: Option<i32>,
    y: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
) -> anyhow::Result<Value> {
    let region = validate_region(x, y, width, height)?;
    if display.is_some() && region.is_some() {
        anyhow::bail!("choose either display or region for a screenshot, not both")
    }
    let path = screenshot_path();
    let mut command = Command::new("screencapture");
    command.args(["-x", "-t", "jpg"]);
    if let Some(display) = display {
        if display == 0 {
            anyhow::bail!("macOS display numbers start at 1")
        }
        command.args(["-D", &display.to_string()]);
    }
    if let Some((x, y, width, height)) = region {
        command.args(["-R", &format!("{x},{y},{width},{height}")]);
    }
    command.arg(&path);
    let output = command.output()?;
    if !output.status.success() {
        let _ = fs::remove_file(&path);
        anyhow::bail!(
            "screenshot failed (Screen Recording permission may be required): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    screenshot_body(&path)
}

#[cfg(target_os = "macos")]
fn macos_focus(app: &str) -> anyhow::Result<Value> {
    ensure_text_limit(app)?;
    let status = Command::new("open").args(["-a", app]).status()?;
    if !status.success() {
        anyhow::bail!("could not activate application {app:?}")
    }
    Ok(json!({"focused": app}))
}

#[cfg(target_os = "macos")]
fn macos_click(x: i32, y: i32, button: Option<&str>) -> anyhow::Result<Value> {
    let button = button.unwrap_or("left").to_ascii_lowercase();
    if button != "left" {
        anyhow::bail!("macOS coordinate click currently supports only the left button")
    }
    const SCRIPT: &str = r#"
on run argv
  set px to item 1 of argv as integer
  set py to item 2 of argv as integer
  tell application "System Events" to click at {px, py}
end run
"#;
    let output = Command::new("osascript")
        .args(["-e", SCRIPT, "--", &x.to_string(), &y.to_string()])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "click failed (Accessibility permission may be required): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    Ok(json!({"clicked": {"x": x, "y": y, "button": button}}))
}

#[cfg(target_os = "macos")]
fn macos_type_text(text: &str) -> anyhow::Result<Value> {
    ensure_text_limit(text)?;
    const SCRIPT: &str = r#"
on run argv
  tell application "System Events" to keystroke (item 1 of argv)
end run
"#;
    let output = Command::new("osascript")
        .args(["-e", SCRIPT, "--", text])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "typing failed (Accessibility permission may be required): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    Ok(json!({"typed_bytes": text.len()}))
}

#[cfg(target_os = "macos")]
fn macos_key(key: &str, modifiers: &[String]) -> anyhow::Result<Value> {
    ensure_text_limit(key)?;
    let using = macos_modifier_clause(modifiers)?;
    let normalized = key.to_ascii_lowercase();
    let key_code = match normalized.as_str() {
        "return" | "enter" => Some(36),
        "tab" => Some(48),
        "space" => Some(49),
        "backspace" | "delete" => Some(51),
        "escape" | "esc" => Some(53),
        "home" => Some(115),
        "end" => Some(119),
        "page_up" | "pageup" => Some(116),
        "page_down" | "pagedown" => Some(121),
        "left" | "arrow_left" => Some(123),
        "right" | "arrow_right" => Some(124),
        "down" | "arrow_down" => Some(125),
        "up" | "arrow_up" => Some(126),
        _ => None,
    };
    let output = if let Some(code) = key_code {
        Command::new("osascript")
            .args([
                "-e",
                &format!("tell application \"System Events\" to key code {code}{using}"),
            ])
            .output()?
    } else {
        if key.chars().count() != 1 {
            anyhow::bail!("unknown macOS key {key:?}")
        }
        let script = format!(
            "on run argv\n tell application \"System Events\" to keystroke (item 1 of argv){using}\nend run"
        );
        Command::new("osascript")
            .args(["-e", &script, "--", key])
            .output()?
    };
    if !output.status.success() {
        anyhow::bail!(
            "key input failed (Accessibility permission may be required): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    Ok(json!({"key": key, "modifiers": modifiers}))
}

#[cfg(target_os = "macos")]
fn macos_modifier_clause(modifiers: &[String]) -> anyhow::Result<String> {
    let mut items = Vec::new();
    for modifier in modifiers {
        let item = match modifier.to_ascii_lowercase().as_str() {
            "command" | "cmd" | "meta" => "command down",
            "control" | "ctrl" => "control down",
            "option" | "alt" => "option down",
            "shift" => "shift down",
            other => anyhow::bail!("unknown macOS modifier {other:?}"),
        };
        if !items.contains(&item) {
            items.push(item);
        }
    }
    if items.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!(" using {{{}}}", items.join(", ")))
    }
}

#[cfg(target_os = "macos")]
fn macos_clipboard_get() -> anyhow::Result<Value> {
    let output = checked_output(Command::new("pbpaste"), "clipboard read")?;
    let text = String::from_utf8(output.stdout)?;
    Ok(json!({"text": text}))
}

#[cfg(target_os = "macos")]
fn macos_clipboard_set(text: &str) -> anyhow::Result<Value> {
    ensure_text_limit(text)?;
    let mut child = Command::new("pbcopy").stdin(Stdio::piped()).spawn()?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("pbcopy stdin unavailable"))?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("clipboard write failed")
    }
    Ok(json!({"written_bytes": text.len()}))
}

#[cfg(windows)]
fn powershell(script: &str, label: &str) -> anyhow::Result<Output> {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-STA",
        "-Command",
        script,
    ]);
    checked_output(command, label)
}

#[cfg(windows)]
fn windows_displays() -> anyhow::Result<Value> {
    let script = r#"Add-Type -AssemblyName System.Windows.Forms; @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { [pscustomobject]@{ device=$_.DeviceName; primary=$_.Primary; x=$_.Bounds.X; y=$_.Bounds.Y; width=$_.Bounds.Width; height=$_.Bounds.Height; working_x=$_.WorkingArea.X; working_y=$_.WorkingArea.Y; working_width=$_.WorkingArea.Width; working_height=$_.WorkingArea.Height } }) | ConvertTo-Json -Compress"#;
    let output = powershell(script, "display discovery")?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    Ok(json!({"platform": "windows", "displays": value}))
}

#[cfg(windows)]
fn windows_windows() -> anyhow::Result<Value> {
    let script = r#"@(Get-Process | Where-Object { $_.MainWindowHandle -ne 0 } | ForEach-Object { [pscustomobject]@{ pid=$_.Id; app=$_.ProcessName; title=$_.MainWindowTitle; handle=$_.MainWindowHandle.ToInt64() } }) | ConvertTo-Json -Compress"#;
    let output = powershell(script, "window discovery")?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    Ok(json!({"platform": "windows", "windows": value}))
}

#[cfg(windows)]
fn windows_screenshot(
    display: Option<u32>,
    x: Option<i32>,
    y: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
) -> anyhow::Result<Value> {
    let region = validate_region(x, y, width, height)?;
    if display.is_some() && region.is_some() {
        anyhow::bail!("choose either display or region for a screenshot, not both")
    }
    let path = screenshot_path();
    let path_text = path.to_string_lossy().replace('\'', "''");
    let bounds = if let Some((x, y, width, height)) = region {
        format!("$x={x};$y={y};$w={width};$h={height};")
    } else if let Some(display) = display {
        format!(
            "$s=[System.Windows.Forms.Screen]::AllScreens[{display}];$x=$s.Bounds.X;$y=$s.Bounds.Y;$w=$s.Bounds.Width;$h=$s.Bounds.Height;"
        )
    } else {
        "$r=[System.Windows.Forms.SystemInformation]::VirtualScreen;$x=$r.X;$y=$r.Y;$w=$r.Width;$h=$r.Height;".into()
    };
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; {bounds} $bmp=New-Object System.Drawing.Bitmap($w,$h); $g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($x,$y,0,0,$bmp.Size); $bmp.Save('{path_text}',[System.Drawing.Imaging.ImageFormat]::Jpeg); $g.Dispose(); $bmp.Dispose();"
    );
    let output = powershell(&script, "screenshot")?;
    drop(output);
    screenshot_body(&path)
}

#[cfg(windows)]
fn windows_focus(app: &str) -> anyhow::Result<Value> {
    ensure_text_limit(app)?;
    let encoded = STANDARD.encode(app.as_bytes());
    let script = format!(
        "$a=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')); $w=New-Object -ComObject WScript.Shell; if(-not $w.AppActivate($a)){{ exit 2 }}"
    );
    powershell(&script, "focus application")?;
    Ok(json!({"focused": app}))
}

#[cfg(windows)]
fn windows_click(x: i32, y: i32, button: Option<&str>) -> anyhow::Result<Value> {
    let (down, up, button) = match button.unwrap_or("left").to_ascii_lowercase().as_str() {
        "left" => (0x0002_u32, 0x0004_u32, "left"),
        "right" => (0x0008_u32, 0x0010_u32, "right"),
        other => anyhow::bail!("unsupported Windows mouse button {other:?}"),
    };
    let script = format!(
        r#"Add-Type @'
using System; using System.Runtime.InteropServices;
public static class InfraMouse {{ [DllImport("user32.dll")] public static extern bool SetCursorPos(int X,int Y); [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint dx,uint dy,uint d,UIntPtr e); }}
'@; [InfraMouse]::SetCursorPos({x},{y}) | Out-Null; [InfraMouse]::mouse_event({down},0,0,0,[UIntPtr]::Zero); [InfraMouse]::mouse_event({up},0,0,0,[UIntPtr]::Zero)"#
    );
    powershell(&script, "mouse click")?;
    Ok(json!({"clicked": {"x": x, "y": y, "button": button}}))
}

#[cfg(windows)]
fn windows_type_text(text: &str) -> anyhow::Result<Value> {
    ensure_text_limit(text)?;
    let escaped = sendkeys_escape(text);
    let encoded = STANDARD.encode(escaped.as_bytes());
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; $t=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')); [System.Windows.Forms.SendKeys]::SendWait($t)"
    );
    powershell(&script, "type text")?;
    Ok(json!({"typed_bytes": text.len()}))
}

#[cfg(windows)]
fn windows_key(key: &str, modifiers: &[String]) -> anyhow::Result<Value> {
    let mut prefix = String::new();
    for modifier in modifiers {
        match modifier.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => {
                if !prefix.contains('^') {
                    prefix.push('^')
                }
            }
            "option" | "alt" => {
                if !prefix.contains('%') {
                    prefix.push('%')
                }
            }
            "shift" => {
                if !prefix.contains('+') {
                    prefix.push('+')
                }
            }
            "command" | "cmd" | "meta" => anyhow::bail!("Windows meta key is not supported yet"),
            other => anyhow::bail!("unknown Windows modifier {other:?}"),
        }
    }
    let normalized = key.to_ascii_lowercase();
    let token = match normalized.as_str() {
        "return" | "enter" => "{ENTER}".into(),
        "tab" => "{TAB}".into(),
        "space" => " ".into(),
        "backspace" => "{BACKSPACE}".into(),
        "delete" => "{DELETE}".into(),
        "escape" | "esc" => "{ESC}".into(),
        "home" => "{HOME}".into(),
        "end" => "{END}".into(),
        "page_up" | "pageup" => "{PGUP}".into(),
        "page_down" | "pagedown" => "{PGDN}".into(),
        "left" | "arrow_left" => "{LEFT}".into(),
        "right" | "arrow_right" => "{RIGHT}".into(),
        "down" | "arrow_down" => "{DOWN}".into(),
        "up" | "arrow_up" => "{UP}".into(),
        _ if key.chars().count() == 1 => sendkeys_escape(key),
        _ => anyhow::bail!("unknown Windows key {key:?}"),
    };
    let sequence = format!("{prefix}{token}");
    let encoded = STANDARD.encode(sequence.as_bytes());
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; $t=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')); [System.Windows.Forms.SendKeys]::SendWait($t)"
    );
    powershell(&script, "key input")?;
    Ok(json!({"key": key, "modifiers": modifiers}))
}

#[cfg(windows)]
fn sendkeys_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("{ENTER}"),
            '\t' => out.push_str("{TAB}"),
            '+' | '^' | '%' | '~' | '(' | ')' | '[' | ']' => {
                out.push('{');
                out.push(ch);
                out.push('}');
            }
            '{' => out.push_str("{{}"),
            '}' => out.push_str("{}}"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(windows)]
fn windows_clipboard_get() -> anyhow::Result<Value> {
    let output = powershell("Get-Clipboard -Raw", "clipboard read")?;
    let text = String::from_utf8(output.stdout)?;
    Ok(json!({"text": text}))
}

#[cfg(windows)]
fn windows_clipboard_set(text: &str) -> anyhow::Result<Value> {
    ensure_text_limit(text)?;
    let encoded = STANDARD.encode(text.as_bytes());
    let script = format!(
        "$t=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')); Set-Clipboard -Value $t"
    );
    powershell(&script, "clipboard write")?;
    Ok(json!({"written_bytes": text.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_region_requires_complete_bounds() {
        assert!(validate_region(None, None, None, None).unwrap().is_none());
        assert_eq!(
            validate_region(Some(1), Some(2), Some(3), Some(4)).unwrap(),
            Some((1, 2, 3, 4))
        );
        assert!(validate_region(Some(1), None, Some(3), Some(4)).is_err());
        assert!(validate_region(Some(1), Some(2), Some(0), Some(4)).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn sendkeys_text_is_escaped() {
        assert_eq!(sendkeys_escape("a+b{c}\n"), "a{+}b{{}c{}}{ENTER}");
    }
}

use crate::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct McpDesktopInput {
    /// Stable device ID or unique name from devices. Check online state and capabilities first.
    pub(crate) device: String,
    /// Choose an allowed action. Only that action is executed; provide its required fields.
    pub(crate) action: DesktopActionName,
    #[serde(default)]
    /// Optional display identifier returned by displays.
    pub(crate) display: Option<u32>,
    #[serde(default)]
    /// Pixel x coordinate; required for click. For a screenshot region supply x, y, width and height together.
    pub(crate) x: Option<i32>,
    #[serde(default)]
    /// Pixel y coordinate; required for click. Screenshot regions require all four bounds.
    pub(crate) y: Option<i32>,
    #[serde(default)]
    /// Screenshot-region width in pixels; requires x, y and height.
    pub(crate) width: Option<u32>,
    #[serde(default)]
    /// Screenshot-region height in pixels; requires x, y and width.
    pub(crate) height: Option<u32>,
    #[serde(default)]
    /// Required application identifier/name for focus. Local desktop permission may be needed.
    pub(crate) app: Option<String>,
    #[serde(default)]
    /// Required text for type_text/clipboard_set. These actions change the device UI or clipboard.
    pub(crate) text: Option<String>,
    #[serde(default)]
    /// Required key name for key; combine with modifiers when needed.
    pub(crate) key: Option<String>,
    #[serde(default)]
    /// Optional modifier names for key; default empty.
    pub(crate) modifiers: Option<Vec<String>>,
    #[serde(default)]
    /// Optional click button; defaults to primary. Availability depends on the platform.
    pub(crate) button: Option<String>,
}

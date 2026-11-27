// Quickshell: one JSON line per change, parsed as it arrives.
import Quickshell
import Quickshell.Io
import QtQuick

Scope {
  id: root
  property string text: ""
  property string state: "offline"
  property string tooltip: ""

  Process {
    running: true
    command: ["podctl", "bar"]
    stdout: SplitParser {
      onRead: line => {
        const o = JSON.parse(line)
        root.text = o.text
        root.state = o.alt
        // waybar separates tooltip lines with a carriage return.
        root.tooltip = (o.tooltip || "").replace(/\r/g, "\n")
      }
    }
  }
}

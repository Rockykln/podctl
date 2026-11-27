// AGS 3 (Astal). `subprocess` keeps the command running and hands over
// one line at a time.
import { subprocess } from "ags/process"
import { createState } from "ags"

type Bar = {
  text: string
  alt: string
  class: string[]
  tooltip: string
  percentage: number | null
}

const empty: Bar = {
  text: "",
  alt: "offline",
  class: ["offline"],
  tooltip: "",
  percentage: null,
}

export const [pods, setPods] = createState<Bar>(empty)

subprocess(
  ["podctl", "bar"],
  (line) => {
    try {
      setPods(JSON.parse(line) as Bar)
    } catch {
      // A half-written line is not worth a crash.
    }
  },
  () => setPods(empty),
)

// In a widget: <label label={pods((p) => p.text)} />
// The exact way a state is read differs between AGS versions — check
// the one you have.

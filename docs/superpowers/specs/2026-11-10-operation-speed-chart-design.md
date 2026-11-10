# Operation speed charts

Copy, Move and Delete show a live, Windows Explorer-inspired area chart in the
selected job detail. Automatic progress and manually opened Jobs use the same
component. Existing completion, cancellation and verified-copy policy remain.

The implementation uses the existing Slint renderer; no plotting dependency or
Windows-only UI is needed. A single average would hide stalls; independent charts
in every queue row would waste space and repaint work. The chosen design is one
compact chart for the selected operation, retaining its entire lifetime.

Backend sampling uses monotonic time and actual cumulative counters at most four
times per second, with at most 240 retained intervals per job. When full, adjacent
intervals are combined and the resolution of future intervals grows equally.
Their original start/end timestamps and each series' measured maximum survive.
The graph's horizontal scale always covers time zero through the latest sample;
its vertical scale includes every historical peak. Each series' peak within an
older combined interval need not have occurred simultaneously. Current labels
still use the latest actual measurement, never a historical bucket maximum.
This bounds memory independently of operation duration. Copy and Move display
written bytes/s plus a distinct verification/readback series (including additional
Move proof reads), in IEC units. These are measured activity, not percentages or
physical storage throughput. Delete displays processed items/s, including reported
folders; moving into the system Trash may report only accepted top-level items.
No size scan is started to invent deletion byte throughput.

Queued/preparing/decision states do not imply data transfer. Stalls settle to zero;
waiting and terminal states show zero current activity while retaining history.
Retry counter resets, cancellation and zero/very short durations must never cause
negative, infinite or fake spikes. Short completed jobs still retain their actual
observed activity. Each job owns its history; switching selection never mixes it.
All sampling and arithmetic stay in the backend. UI code only formats labels and
maps bounded samples to chart geometry. Terminal job history lives with the
existing session-only Jobs records, never in settings.

The chart has a filled transfer/item series, a separate verification line, current
speed labels and a shared vertical scale. Theme semantic colors, high-contrast
outlines and text labels distinguish the series without relying on color alone.
It resizes with the dialog and becomes compact at small heights, preserving the
Close/Cancel/Resolve controls. It must not alter operation authority or checksum
requirements. Tooltips explain the measurement and peak-preserving older intervals.

Verification covers deterministic rates, stalls, reset/retry, pause/resume, bounded
retention across hours, early-peak preservation, short terminal jobs, actual Copy/Move/Delete event routes, and rendered
normal/narrow dialogs with accessible labels and selection stability.

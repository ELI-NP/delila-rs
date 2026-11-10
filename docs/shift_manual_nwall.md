# N_Wall DAQ — Shift Manual

For the shift crew. Everything is done in a web browser: you start and stop runs
and keep the logbook. Experts change settings and fix problems. Everything else is
in the full manual (coming soon).

## Pages

All on the DAQ PC (pf-qaq, 192.168.144.142). Keep the Operator UI and
"Rates — N_Wall" open all the time.

| What | Address |
|---|---|
| Run control (Operator UI) | http://192.168.144.142:9091/ |
| Logbook (eLog) | http://192.168.144.142:8082/N_Wall/ |
| Grafana | http://192.168.144.142:3000/ → Dashboards: **Rates — N_Wall**, **DELILA DAQ Overview**, **HV Monitor (SY5527)**, **Si HV (N1408)** |
| Si HV control (N1408) | http://192.168.144.142:8095/ |

If no address works after a power cut, call the expert: the DAQ PC may have got a new one.

## 1. Start a run

1. Operator UI → **Control**. Before you start, check:
   - **System State**: Configured or Idle.
   - **Component Status → Readers**: "6/6 Online" and a green **PLL OK**.
   - No red box in the Control Panel.

   If one of these is wrong, do not start: see §5.
2. **Comment (for Start)**: what is on: beam or source, target, Si HV on or off,
   anything special. It goes into the run record and the logbook.
3. Leave **Next run** as it is. It counts by itself.
4. Run length: as the run plan says (usually 2 h). To let the DAQ stop by itself,
   tick **Run with timer**, set **Duration (min)** (120 for 2 h, at most 1440) and
   keep **Auto-stop on expire** ticked. The DAQ itself then stops the run on time,
   even if every browser is closed. The time left is shown at the top of every
   page (for example "1:23:45 left"; it may skip a second now and then, that is
   normal).
5. Press **Start**. Within a minute: System State "Running", every reader shows a
   rate, no "No events" box.

   If a box **Waveform Recording Enabled** appears, press **Cancel** and call the
   expert (waveforms make huge files).

## 2. About 15 minutes after Start: screenshots

1. Operator UI → **Monitor**: on each tab press **Save Image** (saves a PNG of what
   is shown). If a spectrum is too spiky to read, first press **×2**, **×4** or
   **×8** next to "Rebin" (2, 4 or 8 channels per bin). It changes the display only,
   never the recorded data.
2. Grafana **Rates — N_Wall**: take a screenshot of the page
   (Ubuntu Shift+PrtSc, Windows Win+Shift+S, macOS Cmd+Shift+4).
3. Logbook → **New**: Author = your name, Type = Routine, Category = DAQ,
   Run Number, Subject "Run N: start check". Attach the images and add one line on
   how it looks.

## 3. During the run (every 2 h, or as the run plan says)

Look at the following, then write one logbook line (Type = Routine):

- Control Panel: no red or yellow box.
- Readers: 6/6 Online, PLL OK, every board has a rate.
- **Rates — N_Wall**: rates like before (no channel suddenly much higher, none
  dropped to 0); the "Digitizer clock (PLL)" row is green.
- **DELILA DAQ Overview**: "Backlog queue per component" near 0 (short peaks are
  fine); "Disk free per writer" not red.
- **HV Monitor (SY5527)** and **Si HV (N1408)**: voltages steady, no trip, currents
  like before.

Short remarks during the run can also go into **Run Notes** (Add Note) in the
Control Panel.

If the run plan changes: Control Panel → **+30 min** or **Cancel timer**. A run
without a timer can get one with **Set timer**. Each change is noted in the run
record by itself.

## 4. Stop a run

1. Press **Stop**, or let the timer do it: then every open page shows
   "Run N: time is up" with an alarm sound; press **Dismiss**. (If Auto-stop was
   not ticked, the box says the run goes on: press Stop yourself.) System State
   goes back to "Configured".
2. The logbook gets a "Run Summary" entry by itself. Reply to it if you have
   something to add.
3. Before the next Start: **DELILA DAQ Overview** → "Backlog queue per component"
   must be back at 0. It usually takes seconds, longer after very high rates.

## 5. If something is wrong

Do not try to fix it. Write what you see and the time in the logbook
(Type = Problem), then call the expert.

| You see | Do |
|---|---|
| Red box "Run N stopped automatically …" | Do not start a new run. Copy the text of the box into the logbook. Call the expert. |
| Readers header "… not OK", or a reader shows "PLL NOT LOCKED", "PLL ?" or an error name such as `jesd_unlock` | Do not start. Call the expert (clock cable, crate or a DAQ restart: experts decide). |
| "Start failed" box | Logbook, then press Start once more. If it fails again, call the expert. |
| "No events" box during the run | Logbook. If it lasts more than 5 min, call the expert. |
| HV trip, or a current jumps | Logbook, call the expert. For the Si HV, see below. |
| A page does not load | Reload it. If the Operator UI stays dead for 5 min, call the expert. |

**Si HV emergency OFF, if possible:** N1408 page → your name and the passphrase (on
the shift sheet) →
**Take control** → **ALL OFF**. The button works only when the N1408 panel is set to
REMOTE. If the page cannot do it (the N1408 is in LOCAL), call the expert at once.
The N1408 page runs on its own: it works even when the DAQ is stopped.

## 6. DAQ restart (only when the expert asks for it)

1. If a run is going, press **Stop**.
2. **DELILA DAQ Overview** → "Backlog queue per component" must be 0 for every
   component. If not, wait: data still waiting there is lost by a restart.
3. On the DAQ PC open a terminal (or `ssh daq@192.168.144.142`) and type:

   ```
   cd ~/delila-rs
   ./scripts/stop_daq.sh
   ./scripts/start_daq.sh
   ```

   Type `start_daq.sh` without anything after it: that starts N_Wall. Never give
   it another config file (for example `config_daq_test_pfqaq.toml` is for tests:
   it writes elsewhere and posts nothing to the logbook).

4. Wait 1 minute and reload the Operator UI: Readers "6/6 Online", **PLL OK**.
   If a board still shows "PLL ?" after 2 minutes, do steps 3 and 4 once more.
   Still bad: call the expert.
5. Start the next run as in §1 (Start also configures the digitizers).

## 7. Never

- Change anything under **Settings**, or use Tune Up.
- Use the ⋮ menu of the Control Panel (Configure, Force Reset) or the run number
  **Override**.
- Restart the DAQ or the PC unless the expert asks for it.
- Touch cables, crates or HV modules (except the Si HV emergency OFF above).
- Delete files or logbook entries.

## Contacts (fill in by hand)

| | Name | Phone |
|---|---|---|
| Expert on call | | |
| Run coordinator | | |

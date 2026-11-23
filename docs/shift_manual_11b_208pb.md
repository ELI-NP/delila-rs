# 11B_208Pb DAQ — Shift Manual

For the shift crew. Everything is done in a web browser: you start and stop runs,
watch the online plots and keep the logbook. Experts change settings and fix
problems.

## Pages

All on the DAQ PC (pf-qaq, 192.168.144.142). Keep the Operator UI,
"Rates — 11B_208Pb" and the ThGEM XY page (§3) open all the time.

| What | Address |
|---|---|
| Run control (Operator UI) | http://192.168.144.142:9091/ |
| Logbook (eLog) | http://192.168.144.142:8082/11B_208Pb/ |
| Grafana | http://192.168.144.142:3000/ → Dashboards → folder **11B_208Pb**: **Rates — 11B_208Pb**, **HV Monitor (SY5527)**; folder Common: **DELILA DAQ Overview** |
| Online plots (ThGEM, beam, spectra) | http://192.168.144.142:8090/ — ready-made pages in §3 |

If no address works after a power cut, call the expert: the DAQ PC may have got a
new one.

## 1. Start a run

1. Operator UI → **Control**. Before you start, check:
   - **System State**: Configured or Idle.
   - **Component Status → Readers**: "5/5 Online" and a green **PLL OK**.
   - No red box in the Control Panel.

   If one of these is wrong, do not start: see §6.
2. **Comment (for Start)**: what is on: beam or source, target, ThGEM HV on or
   off, source motor on or off, anything special. It goes into the run record and
   the logbook.
3. Leave **Next run** as it is. It counts by itself.
4. Run length: as the run plan says. To let the DAQ stop by itself, tick
   **Run with timer**, set **Duration (min)** (120 for 2 h, at most 1440) and keep
   **Auto-stop on expire** ticked. The DAQ itself then stops the run on time, even
   if every browser is closed. The time left is shown at the top of every page
   (for example "1:23:45 left"; it may skip a second now and then, that is
   normal).
5. Clear the online plots (§3), so they show the new run only: open

   ```
   http://192.168.144.142:8090/Reset/cmd.json
   ```

   The page shows just `0`: that is normal, the plots are now empty. Do this just
   before Start, not after it (after Start it would also wipe the first seconds
   of the new run from the plots).
6. Press **Start**. Within a minute: System State "Running", every reader shows a
   rate, no "No events" box.

   If a box **Waveform Recording Enabled** appears, press **Cancel** and call the
   expert (waveforms make huge files).

## 2. About 15 minutes after Start: screenshots

1. Online plots: open the pages **A**, **Ba** and **D** of §3 and take a screenshot of each.
2. Operator UI → **Monitor**: on each tab press **Save Image** (saves a PNG of what
   is shown). If a spectrum is too spiky to read, first press **×2**, **×4** or
   **×8** next to "Rebin". It changes the display only, never the recorded data.
3. Grafana **Rates — 11B_208Pb**: take a screenshot of the page.

   Screenshots: Ubuntu Shift+PrtSc, Windows Win+Shift+S, macOS Cmd+Shift+4.
4. Logbook → **New**: Author = your name, Type = Routine, Category = DAQ,
   Run Number, Subject "Run N: start check". Attach the images and add one line on
   how it looks.

## 3. Online plots

The page at port 8090 draws the histograms of the running DAQ in the browser and
redraws them by itself. Open one of these addresses (copy the whole line):

**A. ThGEM XY images, all eight** (x = t_XR − t_XL, y = t_YU − t_YD, events with
all four arms):

```
http://192.168.144.142:8090/?items=[thgem/xy/xy_T1,thgem/xy/xy_T2,thgem/xy/xy_T3,thgem/xy/xy_T4,thgem/xy/xy_T5,thgem/xy/xy_T6,thgem/xy/xy_T7,thgem/xy/xy_T8]&opts=[colz,colz,colz,colz,colz,colz,colz,colz]&layout=grid4x2&monitoring=10000
```

**B. ThGEM overview**: arms found per ThGEM, ThGEM time against the bunch, ThGEMs
in the same event, hits per channel:

```
http://192.168.144.142:8090/?items=[thgem/narms,thgem/tof,thgem/pair_hits,hits/hitmap]&opts=[colz,colz,colz,colz]&layout=grid2x2&monitoring=5000
```

**C. Energy per board** (board 0 = beam, Gamma and neutron wall; 1–3 = ThGEMs;
4 = Si beam monitors):

```
http://192.168.144.142:8090/?items=[hits/energy_m0,hits/energy_m1,hits/energy_m2,hits/energy_m3,hits/energy_m4,hits/hitmap]&opts=[colz,colz,colz,colz,colz,colz]&layout=grid3x2&monitoring=5000
```

**D. Beam timing** (each channel of board 0 after the beam pulse, and the Gamma
detectors):

```
http://192.168.144.142:8090/?items=[rf/since_beam,rf/gamma_since_beam]&opts=[colz,hist]&layout=grid1x2&monitoring=5000
```

The plain address http://192.168.144.142:8090/ shows the full list on the left:
click a histogram to draw it.

**How often the plots are redrawn:** the number after `monitoring=` in the address
is the time between redraws in milliseconds (5000 = every 5 s). Change it in the
address bar and press Enter. Do not go below 2000: every redraw downloads all the
plots of the page, and page A is big. The plain address redraws every 2 s. The
**Monitoring** tick box on the page stops and restarts the redrawing (it shows
only when the list on the left is shown).

**The plots add up** until they are cleared: at every run start (§1, step 5) with
http://192.168.144.142:8090/Reset/cmd.json. The pages above need no reload: the
next redraw shows the empty plots. The plots are for watching only: clearing them
never touches the recorded data.

## 4. During the run (every 2 h, or as the run plan says)

Look at the following, then write one logbook line (Type = Routine):

- Control Panel: no red or yellow box.
- Readers: 5/5 Online, PLL OK, every board has a rate.
- **Rates — 11B_208Pb**: rates like before (no channel suddenly much higher, none
  dropped to 0); the "Digitizer clock (PLL)" row is green.
- Online plots A and B: the ThGEM images and the arm counts grow like before.
- **DELILA DAQ Overview**: "Backlog queue per component" near 0 (short peaks are
  fine); "Disk free per writer" not red.
- **HV Monitor (SY5527)**: voltages steady, no trip, currents like before.

Short remarks during the run can also go into **Run Notes** (Add Note) in the
Control Panel.

If the run plan changes: Control Panel → **+30 min** or **Cancel timer**. A run
without a timer can get one with **Set timer**. Each change is noted in the run
record by itself.

## 5. Stop a run

1. Press **Stop**, or let the timer do it: then every open page shows
   "Run N: time is up" with an alarm sound; press **Dismiss**. (If Auto-stop was
   not ticked, the box says the run goes on: press Stop yourself.) System State
   goes back to "Configured".
2. The logbook gets a "Run Summary" entry by itself. Reply to it if you have
   something to add.
3. Before the next Start: **DELILA DAQ Overview** → "Backlog queue per component"
   must be back at 0. It usually takes seconds, longer after very high rates.

## 6. If something is wrong

Do not try to fix it. Write what you see and the time in the logbook
(Type = Problem), then call the expert.

| You see | Do |
|---|---|
| Red box "Run N stopped automatically …" (for example "digitizer not answering") | Do not start a new run. Copy the text of the box into the logbook. Call the expert. |
| Readers header "… not OK", or a reader shows "PLL NOT LOCKED", "PLL ?" or an error name | Do not start. Call the expert (clock cable, crate power or a DAQ restart: experts decide). |
| "Start failed" box | Logbook, then press Start once more. If it fails again, call the expert. |
| "No events" box during the run | Logbook. If it lasts more than 5 min, call the expert. |
| HV trip, or a current jumps | Logbook, call the expert. |
| Online plots stop growing during a run | Reload the page. If they still do not grow, logbook and call the expert. The data are still recorded. |
| A page does not load | Reload it. If the Operator UI stays dead for 5 min, call the expert. |

## 7. DAQ restart (only when the expert asks for it)

1. If a run is going, press **Stop**.
2. **DELILA DAQ Overview** → "Backlog queue per component" must be 0 for every
   component. If not, wait: data still waiting there is lost by a restart.
3. On the DAQ PC open a terminal (or `ssh daq@192.168.144.142`) and type:

   ```
   cd ~/delila-rs
   ./scripts/stop_daq.sh
   ./scripts/start_daq.sh
   ```

   Type `start_daq.sh` without anything after it: that starts 11B_208Pb. Never
   give it another config file (for example `config_daq_test_11b_pfqaq.toml` is
   for tests: it writes elsewhere and posts nothing to the logbook).

4. Wait 1 minute and reload the Operator UI: Readers "5/5 Online", **PLL OK**.
   If a board still shows "PLL ?" after 2 minutes, do steps 3 and 4 once more.
   Still bad: call the expert.
5. Start the next run as in §1 (Start also configures the digitizers).

## 8. Never

- Change anything under **Settings** (thresholds and every other digitizer
  setting), or use Tune Up.
- Use the ⋮ menu of the Control Panel (Configure, Force Reset) or the run number
  **Override**.
- Restart the DAQ or the PC unless the expert asks for it.
- Touch cables, crates or HV modules.
- Delete files or logbook entries.

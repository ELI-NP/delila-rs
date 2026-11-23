import { TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideNoopAnimations } from '@angular/platform-browser/animations';
import { ControlPanelComponent } from './control-panel.component';
import { OperatorService } from '../../services/operator.service';
import { AutoStopNotice, RestartResult, SystemStatus, canRestartDaq } from '../../models/types';

describe('ControlPanelComponent: automatic stop banner (TODO 84)', () => {
  const base: SystemStatus = {
    components: [],
    system_state: 'Configured',
    experiment_name: 'N_Wall',
  };
  const notice: AutoStopNotice = {
    cause: 'rate_too_high',
    phase: 'stopping',
    run_number: 45,
    reason: 'Recorder: 21.0 GB waiting to be written',
    started_ms: 1794043048998,
    message: '',
  };

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [ControlPanelComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideNoopAnimations()],
    }).compileComponents();
  });

  function render(status: SystemStatus) {
    TestBed.inject(OperatorService).status.set(status);
    const fixture = TestBed.createComponent(ControlPanelComponent);
    fixture.detectChanges();
    return fixture;
  }

  const banner = (el: HTMLElement) => el.querySelector('[data-testid="auto-stop-banner"]');

  it('shows no banner without a notice', () => {
    expect(banner(render(base).nativeElement)).toBeNull();
  });

  it('says the rate was too high and not to restart while the queue is written', () => {
    const b = banner(render({ ...base, auto_stop: notice }).nativeElement);
    expect(b).not.toBeNull();
    const text = b!.textContent ?? '';
    expect(text).toContain('Run 45 stopped automatically: data rate too high');
    expect(text).toContain('Recorder: 21.0 GB waiting to be written');
    expect(text).toContain('do not stop or restart the DAQ');
    expect(b!.querySelector('.banner-close')).toBeNull();
  });

  it('says the files are complete after the drain and can be dismissed', () => {
    const fixture = render({ ...base, auto_stop: { ...notice, phase: 'stopped' } });
    const el = fixture.nativeElement as HTMLElement;
    const text = banner(el)!.textContent ?? '';
    expect(text).toContain('All queued data was written; the files are complete.');
    expect(text).toContain('raise');

    (banner(el)!.querySelector('.banner-close') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(banner(el)).toBeNull();
  });

  it('names a digitizer that stopped working and says what to check (TODO 85)', () => {
    const b = banner(
      render({
        ...base,
        auto_stop: {
          ...notice,
          cause: 'digitizer_fault',
          phase: 'stopped',
          reason: 'PHA1-V1725-dE-Ring3: clock lost (PLL not locked) for 10 s',
        },
      }).nativeElement,
    );
    const text = b!.textContent ?? '';
    expect(text).toContain('Run 45 stopped automatically: a digitizer stopped working');
    expect(text).toContain('PHA1-V1725-dE-Ring3: clock lost (PLL not locked) for 10 s');
    expect(text).toContain('All queued data was written; the files are complete.');
    expect(text).toContain('clock cable');
    expect(text).toContain('restart the DAQ between runs');
  });

  it('says data was lost before the Recorder and that the files are not complete (TODO 89)', () => {
    const b = banner(
      render({
        ...base,
        auto_stop: {
          ...notice,
          cause: 'data_loss',
          phase: 'stopped',
          reason: 'batches lost on the way to Recorder (3 batches) — sequence gaps, check the network link',
        },
      }).nativeElement,
    );
    const text = b!.textContent ?? '';
    expect(text).toContain('Run 45 stopped: data lost before the Recorder');
    expect(text).toContain('Recorder (3 batches)');
    expect(text).toContain('the files miss the batches');
    expect(text).not.toContain('the files are complete');
    expect(text).toContain('network link');
  });

  it('names a write failure', () => {
    const b = banner(
      render({ ...base, auto_stop: { ...notice, cause: 'write_failure', phase: 'stopped' } })
        .nativeElement,
    );
    expect(b!.textContent).toContain('Run 45 stopped: writing to disk failed');
  });
});

describe('ControlPanelComponent: run timer (TODO 86)', () => {
  const running: SystemStatus = {
    components: [],
    system_state: 'Running',
    experiment_name: 'N_Wall',
    run_info: {
      run_number: 80,
      exp_name: 'N_Wall',
      comment: '',
      start_time: 0,
      elapsed_secs: 0,
      status: 'running',
      stats: { total_events: 0, total_bytes: 0, average_rate: 0 },
      notes: [],
    },
  };

  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [ControlPanelComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideNoopAnimations()],
    }).compileComponents();
    http = TestBed.inject(HttpTestingController);
  });

  function render(status: SystemStatus | null) {
    TestBed.inject(OperatorService).status.set(status);
    const fixture = TestBed.createComponent(ControlPanelComponent);
    fixture.detectChanges();
    return fixture;
  }

  it('sends the timer with Start; the browser no longer counts or stops', () => {
    const fixture = render({ ...running, system_state: 'Configured', run_info: undefined });
    const panel = fixture.componentInstance;
    panel.useTimer.set(true);
    panel.timer.durationMinutes.set(90);
    panel.timer.autoStop.set(false);

    panel.onStart();

    const req = http.expectOne('/api/run/start');
    expect(req.request.body.timer).toEqual({ minutes: 90, auto_stop: false });
  });

  it('sends no timer when Run with timer is not ticked', () => {
    const panel = render({ ...running, system_state: 'Configured', run_info: undefined })
      .componentInstance;
    panel.onStart();
    const req = http.expectOne('/api/run/start');
    expect(req.request.body.timer).toBeUndefined();
  });

  it('shows the Operator countdown and adds 30 min with PUT', () => {
    const fixture = render({
      ...running,
      run_info: {
        ...running.run_info!,
        timer: { minutes: 120, auto_stop: true, remaining_ms: 3_600_000, ends_at_ms: 0 },
      },
    });
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('[data-testid="run-timer"]')?.textContent).toContain(
      '120 min · stops the run',
    );

    (el.querySelector('[data-testid="timer-extend"]') as HTMLButtonElement).click();
    const req = http.expectOne('/api/run/timer');
    expect(req.request.method).toBe('PUT');
    expect(req.request.body).toEqual({ minutes: 150, auto_stop: true });

    (el.querySelector('[data-testid="timer-cancel"]') as HTMLButtonElement).click();
    expect(http.expectOne('/api/run/timer').request.method).toBe('DELETE');
  });

  it('offers Set timer during a run without one', () => {
    const fixture = render(running);
    const panel = fixture.componentInstance;
    panel.timer.durationMinutes.set(45);
    panel.timer.autoStop.set(true);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    (el.querySelector('[data-testid="timer-set"]') as HTMLButtonElement).click();
    const req = http.expectOne('/api/run/timer');
    expect(req.request.method).toBe('PUT');
    expect(req.request.body).toEqual({ minutes: 45, auto_stop: true });
  });
});

describe('ControlPanelComponent: restart the DAQ (TODO 91)', () => {
  const quiet: SystemStatus = {
    components: [
      {
        name: 'Recorder',
        address: '',
        state: 'Configured',
        online: true,
        role: 'pipeline',
        metrics: { queue_bytes: 0 } as never,
      },
      {
        name: 'Merger',
        address: '',
        state: 'Configured',
        online: true,
        role: 'pipeline',
        metrics: { queue_bytes: 0 } as never,
      },
    ],
    system_state: 'Configured',
    experiment_name: '11B_204Pb',
  };
  const result = (over: Partial<RestartResult> = {}): RestartResult => ({
    request_id: '1795',
    started_ms: 1_795_000_000_000,
    finished_ms: 1_795_000_040_000,
    ok: true,
    stage: 'done',
    message: 'DAQ restarted with config.toml: 8 components online, state Idle',
    ...over,
  });

  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [ControlPanelComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideNoopAnimations()],
    }).compileComponents();
    http = TestBed.inject(HttpTestingController);
  });

  function render(status: SystemStatus | null) {
    TestBed.inject(OperatorService).status.set(status);
    const fixture = TestBed.createComponent(ControlPanelComponent);
    fixture.detectChanges();
    return fixture;
  }

  it('is offered between runs only', () => {
    expect(canRestartDaq('Configured', false, false)).toBeTrue();
    expect(canRestartDaq('Idle', false, false)).toBeTrue();
    expect(canRestartDaq('Error', false, false)).toBeTrue();
    expect(canRestartDaq('Degraded', false, false)).toBeTrue();
    expect(canRestartDaq('Running', true, false)).toBeFalse();
    expect(canRestartDaq('Armed', false, false)).toBeFalse();
    expect(canRestartDaq('Configured', true, false)).toBeFalse();
    expect(canRestartDaq('Configured', false, true)).toBeFalse();
    expect(canRestartDaq('Offline', false, false)).toBeFalse();
    expect(render(quiet).componentInstance.restartAllowed()).toBeTrue();
  });

  it('lists queued data as a failed precondition', () => {
    const busy: SystemStatus = {
      ...quiet,
      components: [{ ...quiet.components[0], metrics: { queue_bytes: 12_300_000 } as never }, quiet.components[1]],
    };
    const checks = render(busy).componentInstance.restartChecks();
    const bad = checks.filter((c) => !c.ok).map((c) => c.label);
    expect(bad).toEqual(['Recorder: 12.3 MB still queued']);
  });

  it('waits for the result of its own request and then shows it', () => {
    const fixture = render(quiet);
    const panel = fixture.componentInstance;
    panel.sendRestart();
    const req = http.expectOne('/api/daq/restart');
    expect(req.request.method).toBe('POST');
    req.flush({
      success: true,
      message: 'DAQ restart accepted (request 1795): every component stops and starts again with config.toml.',
    });
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(panel.restartPending()).toEqual({ requestId: '1795' });
    expect(el.querySelector('[data-testid="restart-progress-banner"]')?.textContent).toContain(
      'Restarting the DAQ',
    );

    // An older result (another request) does not end the wait.
    TestBed.inject(OperatorService).status.set({ ...quiet, last_restart: result({ request_id: '1000' }) });
    fixture.detectChanges();
    expect(panel.restartPending()).not.toBeNull();

    TestBed.inject(OperatorService).status.set({ ...quiet, last_restart: result() });
    fixture.detectChanges();
    expect(panel.restartPending()).toBeNull();
    const banner = el.querySelector('[data-testid="restart-result-banner"]');
    expect(banner?.textContent).toContain('DAQ restarted');
    expect(banner?.textContent).toContain('8 components online');
  });

  it('shows a refusal as an error and does not wait', () => {
    const fixture = render(quiet);
    const panel = fixture.componentInstance;
    panel.sendRestart();
    http
      .expectOne('/api/daq/restart')
      .flush(
        { success: false, message: 'DAQ restart refused: Recorder still has 5.0 MB queued' },
        { status: 409, statusText: 'Conflict' },
      );
    expect(panel.restartPending()).toBeNull();
  });

  it('says that nothing was stopped when the preflight refused', () => {
    const fixture = render({
      ...quiet,
      last_restart: result({ ok: false, stage: 'preflight', message: 'Config config.toml does not load' }),
    });
    const text = (fixture.nativeElement as HTMLElement).querySelector('[data-testid="restart-result-banner"]')
      ?.textContent;
    expect(text).toContain('nothing was stopped');
    expect(text).toContain('does not load');
  });

  it('shows the Operator log tail of a failed start', () => {
    const fixture = render({
      ...quiet,
      last_restart: result({ ok: false, stage: 'start', message: 'did not come back', log_tail: 'ERROR bind' }),
    });
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('[data-testid="restart-result-banner"]')?.textContent).toContain(
      'failed at "start"',
    );
    expect(el.querySelector('.restart-log')?.textContent).toContain('ERROR bind');
  });
});

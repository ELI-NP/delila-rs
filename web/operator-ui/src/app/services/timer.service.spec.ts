import { TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting } from '@angular/common/http/testing';
import { TimerService, formatSeconds } from './timer.service';
import { OperatorService } from './operator.service';
import { RunTimerStatus, SystemStatus, TimerExpiredNotice } from '../models/types';

describe('TimerService (TODO 86: the Operator owns the timer)', () => {
  let timer: TimerService;
  let operator: OperatorService;

  const running = (t?: RunTimerStatus): SystemStatus => ({
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
      timer: t,
    },
  });

  const notice = (at_ms: number): TimerExpiredNotice => ({
    run_number: 80,
    minutes: 120,
    auto_stop: true,
    at_ms,
  });

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
    timer = TestBed.inject(TimerService);
    operator = TestBed.inject(OperatorService);
  });

  afterEach(() => timer.dismissAlarm());

  it('has no timer without one in run_info', () => {
    operator.status.set(running());
    expect(timer.isRunning()).toBeFalse();
    expect(timer.headerText()).toBe('');
  });

  it('counts down from the Operator remaining_ms since the status arrived', () => {
    operator.status.set(
      running({ minutes: 2, auto_stop: true, remaining_ms: 90_000, ends_at_ms: 0 }),
    );
    operator.statusReceivedAt.set(1_000);
    timer.now.set(31_000);
    expect(timer.isRunning()).toBeTrue();
    expect(timer.remainingMs()).toBe(60_000);
    expect(timer.headerText()).toBe('1:00 left');
    expect(timer.progress()).toBe(50);
  });

  it('never counts up when the tick is older than the status', () => {
    operator.status.set(
      running({ minutes: 2, auto_stop: true, remaining_ms: 90_000, ends_at_ms: 0 }),
    );
    operator.statusReceivedAt.set(5_000);
    timer.now.set(4_000);
    expect(timer.remainingMs()).toBe(90_000);
  });

  it('shows the time over for an alarm-only timer', () => {
    operator.status.set(
      running({ minutes: 1, auto_stop: false, remaining_ms: -65_000, ends_at_ms: 0 }),
    );
    operator.statusReceivedAt.set(0);
    timer.now.set(0);
    expect(timer.headerText()).toBe('+1:05 over');
    expect(timer.progress()).toBe(100);
  });

  it('formats hours', () => {
    expect(formatSeconds(2 * 3600 + 3 * 60 + 4)).toBe('2:03:04');
    expect(formatSeconds(59)).toBe('0:59');
  });

  it('alarms for an expiry that appears after the first status, once', () => {
    timer.observe(null);
    expect(timer.alarm()).toBeNull();
    timer.observe(notice(1000));
    expect(timer.alarm()).toEqual(notice(1000));
    timer.dismissAlarm();
    timer.observe(notice(1000));
    expect(timer.alarm()).toBeNull();
    timer.observe(notice(2000));
    expect(timer.alarm()).toEqual(notice(2000));
  });

  it('does not alarm for an expiry that was already there when the page opened', () => {
    timer.observe(notice(1000));
    expect(timer.alarm()).toBeNull();
  });
});

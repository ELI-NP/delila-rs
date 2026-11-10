import { ComponentFixture, TestBed, discardPeriodicTasks, fakeAsync, tick } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting } from '@angular/common/http/testing';
import { NoopAnimationsModule } from '@angular/platform-browser/animations';
import { provideRouter } from '@angular/router';
import { App } from './app';
import { OperatorService } from './services/operator.service';
import { SystemStatus } from './models/types';
import { MatDialog } from '@angular/material/dialog';

describe('App', () => {
  let component: App;
  let fixture: ComponentFixture<App>;
  let operatorService: OperatorService;

  const mockSystemStatus: SystemStatus = {
    components: [
      {
        name: 'Reader-0',
        address: 'tcp://localhost:5555',
        state: 'Running',
        run_number: 42,
        metrics: {
          events_processed: 1000000,
          bytes_transferred: 50000,
          queue_size: 10,
          queue_max: 100,
          event_rate: 1500000,
        },
        online: true,
        role: 'source',
      },
    ],
    system_state: 'Running',
    experiment_name: 'TestExp',
    next_run_number: 43,
  };

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [App, NoopAnimationsModule],
      providers: [
        OperatorService,
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
      ],
    }).compileComponents();

    operatorService = TestBed.inject(OperatorService);
    // Mock startPolling to prevent actual HTTP calls
    spyOn(operatorService, 'startPolling').and.stub();

    fixture = TestBed.createComponent(App);
    component = fixture.componentInstance;
    fixture.detectChanges();
  });

  it('should create the app', () => {
    expect(component).toBeTruthy();
  });

  it('should have correct title in toolbar', () => {
    const toolbar = fixture.nativeElement.querySelector('mat-toolbar');
    expect(toolbar.textContent).toContain('DELILA DAQ');
  });

  it('should call startPolling on init', () => {
    expect(operatorService.startPolling).toHaveBeenCalled();
  });

  it('should show Online when status is available', () => {
    operatorService.status.set(mockSystemStatus);
    fixture.detectChanges();

    const statusIndicator = fixture.nativeElement.querySelector('.status-indicator');
    expect(statusIndicator.textContent.trim()).toBe('Online');
    expect(statusIndicator.classList.contains('online')).toBeTrue();
  });

  it('should show Offline when status is null', () => {
    operatorService.status.set(null);
    fixture.detectChanges();

    const statusIndicator = fixture.nativeElement.querySelector('.status-indicator');
    expect(statusIndicator.textContent.trim()).toBe('Offline');
    expect(statusIndicator.classList.contains('offline')).toBeTrue();
  });

  it('should display tab navigation', () => {
    const tabLinks = fixture.nativeElement.querySelectorAll('[mat-tab-link]');
    expect(tabLinks.length).toBe(5);
    expect(tabLinks[0].textContent.trim()).toBe('Control');
    expect(tabLinks[1].textContent.trim()).toBe('Monitor');
    expect(tabLinks[2].textContent.trim()).toBe('Waveform');
    expect(tabLinks[3].textContent.trim()).toBe('Settings');
    expect(tabLinks[4].textContent.trim()).toBe('Runs');
  });

  it('should display system state in header', () => {
    operatorService.status.set(mockSystemStatus);
    fixture.detectChanges();

    const headerStats = fixture.nativeElement.querySelector('.header-stats');
    expect(headerStats.textContent).toContain('Running');
  });

  it('should format events correctly', () => {
    expect(component.formatEvents(1234567)).toBe('1.23M');
    expect(component.formatEvents(12345)).toBe('12.3K');
    expect(component.formatEvents(123)).toBe('123');
  });

  it('should format rate correctly', () => {
    expect(component.formatRate(1500000)).toBe('1.50M eve/s');
    expect(component.formatRate(15000)).toBe('15.0k eve/s');
    expect(component.formatRate(150)).toBe('150 eve/s');
  });

  it('should display run number when available', () => {
    operatorService.status.set(mockSystemStatus);
    fixture.detectChanges();

    const runInfo = fixture.nativeElement.querySelector('.run-info');
    expect(runInfo).toBeTruthy();
    expect(runInfo.textContent).toContain('Run: 42');
  });

  it('shows the run timer on every page, from the Operator (TODO 86)', () => {
    const timed = (remaining_ms: number, auto_stop: boolean): SystemStatus => ({
      ...mockSystemStatus,
      run_info: {
        run_number: 42,
        exp_name: 'TestExp',
        comment: '',
        start_time: 0,
        elapsed_secs: 0,
        status: 'running',
        stats: { total_events: 0, total_bytes: 0, average_rate: 0 },
        notes: [],
        timer: { minutes: 120, auto_stop, remaining_ms, ends_at_ms: 0 },
      },
    });
    const header = () =>
      fixture.nativeElement.querySelector('[data-testid="header-run-timer"]') as HTMLElement | null;

    fixture.detectChanges();
    expect(header()).toBeNull();

    operatorService.status.set(timed(5_025_000, true));
    operatorService.statusReceivedAt.set(component.timer.now());
    fixture.detectChanges();
    expect(header()!.textContent).toContain('1:23:45 left');
    expect(header()!.classList).not.toContain('over');

    operatorService.status.set(timed(-192_000, false));
    operatorService.statusReceivedAt.set(component.timer.now());
    fixture.detectChanges();
    expect(header()!.textContent).toContain('+3:12 over');
    expect(header()!.classList).toContain('over');
  });

  it('opens the run-timer alarm on any page and clears it on Dismiss (TODO 86)', fakeAsync(() => {
    const dialog = TestBed.inject(MatDialog);
    component.timer.observe(null);
    component.timer.observe({ run_number: 42, minutes: 120, auto_stop: true, at_ms: 1000 });
    TestBed.tick();
    fixture.detectChanges();

    const ref = dialog.getDialogById('run-timer-alarm');
    expect(ref).toBeTruthy();
    expect(document.body.textContent).toContain('Run 42: time is up');
    expect(document.body.textContent).toContain('The Operator stopped the run.');

    ref!.close();
    tick(500);
    fixture.detectChanges();
    expect(component.timer.alarm()).toBeNull();
    discardPeriodicTasks();
  }));
});

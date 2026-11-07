import { TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting } from '@angular/common/http/testing';
import { provideNoopAnimations } from '@angular/platform-browser/animations';
import { ControlPanelComponent } from './control-panel.component';
import { OperatorService } from '../../services/operator.service';
import { AutoStopNotice, SystemStatus } from '../../models/types';

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

  it('names a write failure', () => {
    const b = banner(
      render({ ...base, auto_stop: { ...notice, cause: 'write_failure', phase: 'stopped' } })
        .nativeElement,
    );
    expect(b!.textContent).toContain('Run 45 stopped: writing to disk failed');
  });
});

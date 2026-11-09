import { TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting } from '@angular/common/http/testing';
import { provideNoopAnimations } from '@angular/platform-browser/animations';
import { StatusPanelComponent } from './status-panel.component';
import { OperatorService } from '../../services/operator.service';
import { ComponentStatus, PllStatus, SystemStatus } from '../../models/types';

describe('StatusPanelComponent: digitizer clock (PLL) status (TODO 85)', () => {
  const reader = (name: string, pll?: PllStatus): ComponentStatus => ({
    name,
    address: 'tcp://localhost:7101',
    state: 'Running',
    metrics: {
      events_processed: 10,
      bytes_transferred: 0,
      queue_size: 0,
      queue_max: 0,
      event_rate: 1,
      pll,
    },
    online: true,
    role: 'source',
  });
  const status = (components: ComponentStatus[]): SystemStatus => ({
    components,
    system_state: 'Running',
    experiment_name: 'N_Wall',
  });

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [StatusPanelComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideNoopAnimations()],
    }).compileComponents();
  });

  function render(s: SystemStatus) {
    TestBed.inject(OperatorService).status.set(s);
    const fixture = TestBed.createComponent(StatusPanelComponent);
    fixture.detectChanges();
    // The auto-expand effect runs during the first pass; render its result.
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  }

  const header = (el: HTMLElement) => el.querySelector('[data-testid="pll-summary"]');
  const chips = (el: HTMLElement) =>
    Array.from(el.querySelectorAll('[data-testid="pll-chip"]')).map((c) => ({
      text: (c.textContent ?? '').trim(),
      bad: c.classList.contains('pll-bad'),
    }));

  it('says PLL OK in the collapsed header when every clock is locked', () => {
    const el = render(status([reader('A', 'locked'), reader('B', 'locked')]));
    expect(header(el)!.textContent).toContain('PLL OK');
    expect(header(el)!.classList).not.toContain('pll-bad');
  });

  it('counts the boards whose clock is not locked and opens the list', () => {
    const el = render(
      status([reader('E-Sector', 'locked'), reader('Ring3', 'unlocked'), reader('Ring1', 'unreadable')]),
    );
    expect(header(el)!.textContent).toContain('PLL 2 not locked');
    expect(header(el)!.classList).toContain('pll-bad');
    expect(chips(el)).toEqual([
      { text: 'PLL', bad: false },
      { text: 'PLL NOT LOCKED', bad: true },
      { text: 'PLL ?', bad: true },
    ]);
  });

  it('shows nothing for boards without a PLL status', () => {
    const el = render(status([reader('AMax')]));
    expect(header(el)).toBeNull();
    expect(chips(el)).toEqual([]);
  });
});

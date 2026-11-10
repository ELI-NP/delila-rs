import { TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting } from '@angular/common/http/testing';
import { provideNoopAnimations } from '@angular/platform-browser/animations';
import { SetupTabComponent } from './setup-tab.component';
import { SetupConfig } from '../../models/histogram.types';

describe('SetupTabComponent rebin (TODO 87)', () => {
  const base: SetupConfig = {
    name: 'LS',
    gridRows: 1,
    gridCols: 1,
    xAxisLabel: 'Channel',
    histogramType: 'energy',
    cells: [{ sourceId: 0, channelId: 8 }],
  };

  function create(config: SetupConfig) {
    TestBed.configureTestingModule({
      imports: [SetupTabComponent],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideNoopAnimations()],
    });
    const fixture = TestBed.createComponent(SetupTabComponent);
    fixture.componentInstance.config = config;
    const emitted: SetupConfig[] = [];
    fixture.componentInstance.configChange.subscribe((c) => emitted.push(c));
    fixture.detectChanges();
    return { fixture, emitted };
  }

  // How the pf-qaq LS tab became x64: the old Bins field showed the default 1024
  // and a range edit saved it.
  it('saves a range edit without a bin count', () => {
    const { fixture, emitted } = create(base);
    fixture.componentInstance.onXMaxChange('8192');
    const view = emitted[emitted.length - 1].xView!;
    expect(view.max).toBe(8192);
    expect(view.bins).toBeUndefined();
    expect(view.rebin).toBeUndefined();
  });

  it('saves the factor picked, dropping an old bin count', () => {
    const { fixture, emitted } = create({ ...base, xView: { min: 0, max: 65536, bins: 1024 } });
    fixture.componentInstance.onXRebin(4);
    expect(emitted[emitted.length - 1].xView).toEqual({ min: 0, max: 65536, rebin: 4 });
  });

  it('shows the x1..x8 buttons instead of a Bins field', () => {
    const el = create(base).fixture.nativeElement as HTMLElement;
    expect(el.querySelector('app-rebin-picker')).not.toBeNull();
    expect(el.textContent).not.toContain('Bins');
  });
});

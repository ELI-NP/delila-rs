import { TestBed } from '@angular/core/testing';
import { RebinPickerComponent } from './rebin-picker.component';

describe('RebinPickerComponent (TODO 87)', () => {
  function render(value: number, allowed: number[]) {
    const fixture = TestBed.createComponent(RebinPickerComponent);
    fixture.componentRef.setInput('value', value);
    fixture.componentRef.setInput('allowed', allowed);
    fixture.componentRef.setInput('nativeBins', 1500);
    fixture.detectChanges();
    return fixture;
  }
  const button = (el: HTMLElement, f: number) =>
    el.querySelector(`[data-testid="rebin-${f}"]`) as HTMLButtonElement;

  it('shows x1 to x8, marks the one in use, disables what the range cannot split', () => {
    const el = render(2, [1, 2, 4]).nativeElement as HTMLElement;
    expect(Array.from(el.querySelectorAll('button')).map((b) => b.textContent?.trim())).toEqual([
      '×1',
      '×2',
      '×4',
      '×8',
    ]);
    expect(button(el, 2).classList).toContain('active');
    expect(button(el, 8).disabled).toBeTrue();
    expect(button(el, 4).title).toBe('375 bins, 4 native bins per bin');
  });

  it('emits a new factor, not the one in use', () => {
    const fixture = render(1, [1, 2, 4, 8]);
    const picked: number[] = [];
    fixture.componentInstance.valueChange.subscribe((f) => picked.push(f));
    const el = fixture.nativeElement as HTMLElement;
    button(el, 1).click();
    button(el, 8).click();
    expect(picked).toEqual([8]);
  });
});

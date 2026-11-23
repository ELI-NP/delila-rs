import { TestBed } from '@angular/core/testing';
import { MAT_DIALOG_DATA, MatDialogRef } from '@angular/material/dialog';
import { provideNoopAnimations } from '@angular/platform-browser/animations';
import { RestartDaqDialogComponent } from './restart-daq-dialog.component';

describe('RestartDaqDialogComponent (TODO 91)', () => {
  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [RestartDaqDialogComponent],
      providers: [
        provideNoopAnimations(),
        {
          provide: MAT_DIALOG_DATA,
          useValue: {
            checks: [
              { label: 'No run in progress', ok: true },
              { label: 'Recorder: 5.0 MB still queued', ok: false },
            ],
          },
        },
        { provide: MatDialogRef, useValue: { close: () => {} } },
      ],
    }).compileComponents();
  });

  it('lists the preconditions and enables Restart only after RESTART is typed', async () => {
    const fixture = TestBed.createComponent(RestartDaqDialogComponent);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelectorAll('[data-testid="check-ok"]').length).toBe(1);
    expect(el.querySelector('[data-testid="check-bad"]')?.textContent).toContain('5.0 MB still queued');

    const button = el.querySelector('[data-testid="restart-confirm-button"]') as HTMLButtonElement;
    expect(button.disabled).toBeTrue();

    fixture.componentInstance.typed.set('restart');
    fixture.detectChanges();
    expect(button.disabled).toBeTrue();

    fixture.componentInstance.typed.set('RESTART');
    fixture.detectChanges();
    expect(button.disabled).toBeFalse();
  });
});

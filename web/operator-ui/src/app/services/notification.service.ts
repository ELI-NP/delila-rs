import { Component, Inject, Injectable, inject, signal } from '@angular/core';
import {
  MAT_SNACK_BAR_DATA,
  MatSnackBar,
  MatSnackBarConfig,
  MatSnackBarRef,
} from '@angular/material/snack-bar';
import { MatButtonModule } from '@angular/material/button';

export type NotificationType = 'success' | 'error' | 'warning' | 'info';

interface ErrorSnackBarData {
  message: string;
}

@Component({
  selector: 'app-error-snackbar',
  standalone: true,
  imports: [MatButtonModule],
  template: `
    <div class="error-snackbar">
      <span class="message">{{ data.message }}</span>
      <div class="actions">
        <button mat-button (click)="copy()">{{ copied() ? 'Copied' : 'Copy' }}</button>
        <button mat-button (click)="snackBarRef.dismiss()">Dismiss</button>
      </div>
    </div>
  `,
  styles: [`
    .error-snackbar {
      display: flex;
      align-items: flex-start;
      gap: 12px;
      color: white;
      font-size: 14px;
      line-height: 1.4;
    }
    .message {
      flex: 1;
      white-space: pre-wrap;
      word-break: break-word;
    }
    .actions {
      display: flex;
      gap: 4px;
      flex-shrink: 0;
      align-self: center;
    }
    button {
      color: white !important;
      min-width: 0;
      padding: 0 12px;
    }
  `],
})
export class ErrorSnackBarComponent {
  protected readonly copied = signal(false);

  constructor(
    public snackBarRef: MatSnackBarRef<ErrorSnackBarComponent>,
    @Inject(MAT_SNACK_BAR_DATA) public data: ErrorSnackBarData,
  ) {}

  copy(): void {
    const text = this.data.message;
    if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
      navigator.clipboard.writeText(text).then(
        () => this.flashCopied(),
        () => this.fallbackCopy(text),
      );
      return;
    }
    this.fallbackCopy(text);
  }

  private fallbackCopy(text: string): void {
    if (typeof document === 'undefined') {
      return;
    }
    const textarea = document.createElement('textarea');
    textarea.value = text;
    textarea.style.position = 'fixed';
    textarea.style.opacity = '0';
    document.body.appendChild(textarea);
    textarea.select();
    try {
      document.execCommand('copy');
      this.flashCopied();
    } catch {
      // noop — best effort
    } finally {
      document.body.removeChild(textarea);
    }
  }

  private flashCopied(): void {
    this.copied.set(true);
    setTimeout(() => this.copied.set(false), 1500);
  }
}

@Injectable({
  providedIn: 'root',
})
export class NotificationService {
  private readonly snackBar = inject(MatSnackBar);

  private readonly defaultConfig: MatSnackBarConfig = {
    duration: 3000,
    horizontalPosition: 'center',
    verticalPosition: 'bottom',
  };

  show(message: string, type: NotificationType = 'info', action?: string): void {
    if (type === 'error') {
      this.openErrorSnackBar(message);
      return;
    }

    const closeAction = action ?? 'Close';
    const config: MatSnackBarConfig = {
      ...this.defaultConfig,
      panelClass: this.getPanelClass(type),
    };

    this.snackBar.open(message, closeAction, config);
  }

  success(message: string): void {
    this.show(message, 'success');
  }

  error(message: string): void {
    this.openErrorSnackBar(message);
  }

  warning(message: string): void {
    this.show(message, 'warning');
  }

  info(message: string): void {
    this.show(message, 'info');
  }

  private openErrorSnackBar(message: string): void {
    this.snackBar.openFromComponent(ErrorSnackBarComponent, {
      horizontalPosition: 'center',
      verticalPosition: 'bottom',
      panelClass: ['snackbar-error'],
      data: { message },
    });
  }

  private getPanelClass(type: NotificationType): string[] {
    switch (type) {
      case 'success':
        return ['snackbar-success'];
      case 'error':
        return ['snackbar-error'];
      case 'warning':
        return ['snackbar-warning'];
      case 'info':
      default:
        return ['snackbar-info'];
    }
  }
}

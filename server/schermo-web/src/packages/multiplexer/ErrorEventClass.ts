// @ts-nocheck - imitazione di WebSocket scritta per le vecchie tipizzazioni del DOM, provata dal flusso vero
import { Event2 } from './Event';

export class ErrorEvent2 extends Event2 implements ErrorEvent {
    readonly colno: number;
    readonly error: any;
    readonly filename: string;
    readonly lineno: number;
    readonly message: string;

    constructor(type: string, { colno, error, filename, lineno, message }: ErrorEventInit = {}) {
        super(type);
        this.error = error;
        this.colno = colno || 0;
        this.filename = filename || '';
        this.lineno = lineno || 0;
        this.message = message || '';
    }
}

export const ErrorEventClass = typeof ErrorEvent !== 'undefined' ? ErrorEvent : ErrorEvent2;

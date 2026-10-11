import { Buffer } from 'buffer';
import { ControlMessage, ControlMessageInterface } from './ControlMessage';

export interface TextControlMessageInterface extends ControlMessageInterface {
    text: string;
}

export class TextControlMessage extends ControlMessage {
    private static TEXT_SIZE_FIELD_LENGTH = 4;
    constructor(readonly text: string) {
        super(ControlMessage.TYPE_TEXT);
    }

    public getText(): string {
        return this.text;
    }

    /**
     * @override
     */
    public toBuffer(): Buffer {
        // la lunghezza è in byte UTF-8: con un carattere fuori da ASCII non coincide con text.length
        const bytes = Buffer.from(this.text, 'utf8');
        const buffer = Buffer.alloc(bytes.length + 1 + TextControlMessage.TEXT_SIZE_FIELD_LENGTH);
        let offset = 0;
        offset = buffer.writeUInt8(this.type, offset);
        offset = buffer.writeUInt32BE(bytes.length, offset);
        bytes.copy(buffer, offset);
        return buffer;
    }

    public toString(): string {
        return `TextControlMessage{text=${this.text}}`;
    }

    public toJSON(): TextControlMessageInterface {
        return {
            type: this.type,
            text: this.text,
        };
    }
}

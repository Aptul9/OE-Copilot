import '../../../style/help.css';

// Promemoria dei comandi della pagina del telefono, dietro il pulsante "?" della barra. Testo fisso: se cambia il
// comportamento (KeyText.ts, interactionHandler, la barra) si cambia anche qui.
const CONTENT = `
<h3>Phone page: what each thing does</h3>

<h4>Mouse</h4>
<ul>
    <li><kbd>click</kbd> tap, <kbd>drag</kbd> swipe, <kbd>wheel</kbd> scroll</li>
    <li><kbd>Ctrl</kbd> + <kbd>drag</kbd> pinch around the centre, <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>drag</kbd> pinch around the pointer</li>
</ul>

<h4>Keyboard (the keyboard button is highlighted = on)</h4>
<ul>
    <li>Letters, digits and symbols are typed as you see them, any keyboard layout. <kbd>€</kbd> <kbd>£</kbd> and emoji do not exist on the phone and are dropped</li>
    <li><kbd>Ctrl</kbd> + <kbd>V</kbd> pastes your computer clipboard into the phone</li>
    <li><kbd>Enter</kbd> <kbd>Backspace</kbd> <kbd>Delete</kbd> <kbd>Tab</kbd> <kbd>Esc</kbd> <kbd>←</kbd> <kbd>↑</kbd> <kbd>→</kbd> <kbd>↓</kbd> <kbd>Home</kbd> <kbd>End</kbd> <kbd>PgUp</kbd> <kbd>PgDn</kbd> go to the phone as keys</li>
    <li><kbd>Ctrl</kbd> <kbd>Alt</kbd> and <kbd>F1</kbd>..<kbd>F12</kbd> stay with the browser (reload, fullscreen...)</li>
    <li>Keys act on the app in front: <kbd>Space</kbd> or <kbd>Enter</kbd> on a focused button presses it</li>
</ul>

<h4>Buttons, top to bottom</h4>
<ul>
    <li><kbd>?</kbd> this help</li>
    <li><kbd>...</kbd> a text box for logins and long text (<em>Send as keys</em>), notification panel, rotate, video settings</li>
    <li><kbd>Power</kbd> wakes the screen, or sleeps it: a sleeping phone locks and asks for the PIN again. <kbd>Vol +</kbd> <kbd>Vol -</kbd> volume</li>
    <li><kbd>Back</kbd> <kbd>Home</kbd> <kbd>Overview</kbd> the Android navigation buttons</li>
    <li>Camera takes a screenshot, the corners button goes fullscreen</li>
    <li><kbd>Keyboard</kbd> turns the computer keyboard on or off for the phone</li>
</ul>

<h4>Unlock and login</h4>
<ul>
    <li>Screen dark: <kbd>Power</kbd>. Then drag up from the middle of the lock screen, and click the pad or type the PIN</li>
    <li>Login: click the field, type or paste. A long or ready-made text: <kbd>...</kbd>, write it in the box, <em>Send as keys</em></li>
</ul>
`;

export class GoogHelpBox {
    private readonly holder: HTMLElement;

    constructor() {
        const box = document.createElement('div');
        box.className = 'help-box';
        box.innerHTML = CONTENT;
        this.holder = box;
    }

    public getHolderElement(): HTMLElement {
        return this.holder;
    }
}

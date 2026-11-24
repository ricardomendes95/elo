package io.github.ricardomendes95.elo.ui.session

import android.view.KeyEvent

/**
 * Códigos de tecla virtual do Windows (VK_*) usados pela sessão, e o mapeamento
 * das teclas do Android para eles (mesma tabela do desktop, `crates/gui/src/keymap.rs`).
 * Texto que o VK não expressa vai como `Char` (escalar Unicode).
 */
object Vk {
    const val BACK = 0x08
    const val TAB = 0x09
    const val RETURN = 0x0D
    const val SHIFT = 0x10
    const val CONTROL = 0x11
    const val MENU = 0x12 // Alt
    const val PAUSE = 0x13
    const val CAPITAL = 0x14
    const val ESCAPE = 0x1B
    const val SPACE = 0x20
    const val PRIOR = 0x21 // Page Up
    const val NEXT = 0x22 // Page Down
    const val END = 0x23
    const val HOME = 0x24
    const val LEFT = 0x25
    const val UP = 0x26
    const val RIGHT = 0x27
    const val DOWN = 0x28
    const val SNAPSHOT = 0x2C // Print Screen
    const val INSERT = 0x2D
    const val DELETE = 0x2E
    const val LWIN = 0x5B
    const val APPS = 0x5D
    const val F1 = 0x70
    const val NUMLOCK = 0x90
    const val SCROLL = 0x91
    const val VOLUME_MUTE = 0xAD
    const val VOLUME_DOWN = 0xAE
    const val VOLUME_UP = 0xAF

    fun f(n: Int) = F1 + (n - 1)

    /** VK de uma letra/dígito ASCII (para atalhos com modificador, ex.: Ctrl+C). */
    fun forAscii(c: Char): Int? = when (c) {
        in 'a'..'z' -> c.uppercaseChar().code
        in 'A'..'Z' -> c.code
        in '0'..'9' -> c.code
        ' ' -> SPACE
        else -> null
    }

    /** VK para um `KeyEvent.KEYCODE_*` do Android (teclado físico/IME), se houver. */
    fun fromKeyCode(code: Int): Int? = when (code) {
        in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> 0x41 + (code - KeyEvent.KEYCODE_A)
        in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 -> 0x30 + (code - KeyEvent.KEYCODE_0)
        in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> F1 + (code - KeyEvent.KEYCODE_F1)
        KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> RETURN
        KeyEvent.KEYCODE_DEL -> BACK
        KeyEvent.KEYCODE_FORWARD_DEL -> DELETE
        KeyEvent.KEYCODE_TAB -> TAB
        KeyEvent.KEYCODE_SPACE -> SPACE
        KeyEvent.KEYCODE_ESCAPE -> ESCAPE
        KeyEvent.KEYCODE_DPAD_LEFT -> LEFT
        KeyEvent.KEYCODE_DPAD_RIGHT -> RIGHT
        KeyEvent.KEYCODE_DPAD_UP -> UP
        KeyEvent.KEYCODE_DPAD_DOWN -> DOWN
        KeyEvent.KEYCODE_MOVE_HOME -> HOME
        KeyEvent.KEYCODE_MOVE_END -> END
        KeyEvent.KEYCODE_PAGE_UP -> PRIOR
        KeyEvent.KEYCODE_PAGE_DOWN -> NEXT
        KeyEvent.KEYCODE_INSERT -> INSERT
        KeyEvent.KEYCODE_SHIFT_LEFT, KeyEvent.KEYCODE_SHIFT_RIGHT -> SHIFT
        KeyEvent.KEYCODE_CTRL_LEFT, KeyEvent.KEYCODE_CTRL_RIGHT -> CONTROL
        KeyEvent.KEYCODE_ALT_LEFT, KeyEvent.KEYCODE_ALT_RIGHT -> MENU
        KeyEvent.KEYCODE_META_LEFT, KeyEvent.KEYCODE_META_RIGHT -> LWIN
        KeyEvent.KEYCODE_CAPS_LOCK -> CAPITAL
        KeyEvent.KEYCODE_NUM_LOCK -> NUMLOCK
        KeyEvent.KEYCODE_SCROLL_LOCK -> SCROLL
        KeyEvent.KEYCODE_BREAK -> PAUSE
        KeyEvent.KEYCODE_SYSRQ -> SNAPSHOT
        KeyEvent.KEYCODE_MENU -> APPS
        KeyEvent.KEYCODE_MINUS -> 0xBD
        KeyEvent.KEYCODE_EQUALS -> 0xBB
        KeyEvent.KEYCODE_SEMICOLON -> 0xBA
        KeyEvent.KEYCODE_COMMA -> 0xBC
        KeyEvent.KEYCODE_PERIOD -> 0xBE
        KeyEvent.KEYCODE_SLASH -> 0xBF
        KeyEvent.KEYCODE_BACKSLASH -> 0xDC
        KeyEvent.KEYCODE_LEFT_BRACKET -> 0xDB
        KeyEvent.KEYCODE_RIGHT_BRACKET -> 0xDD
        KeyEvent.KEYCODE_GRAVE -> 0xC0
        KeyEvent.KEYCODE_APOSTROPHE -> 0xDE
        else -> null
    }
}

package io.github.ricardomendes95.elo.ui.session

import android.content.Context
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import io.github.ricardomendes95.elo.ffi.Session

/**
 * Traduz o teclado do Android para o protocolo do Elo:
 * - texto do IME → um `Char` por code point (ç, á, €, emojis…);
 * - teclas de controle (Enter, Backspace, setas, F1…) → `Key` com VK do Windows;
 * - modificadores da barra de teclas especiais (Ctrl/Alt/Shift/Win) valem para a
 *   próxima tecla (uma vez) ou ficam travados; com modificador ativo, letras e
 *   dígitos vão como VK (para atalhos como Ctrl+C), não como `Char`.
 */
class RemoteKeyboard(private val session: Session, private val canControl: () -> Boolean) {
    /** Modificadores ativos (VK) → travado? (false = só a próxima tecla). */
    private val mods = linkedMapOf<Int, Boolean>()
    var onModsChanged: ((Map<Int, Boolean>) -> Unit)? = null

    fun modifiers(): Map<Int, Boolean> = mods.toMap()

    /** Toque no modificador: desligado → uma vez → travado → desligado. */
    fun toggleModifier(vk: Int) {
        when (mods[vk]) {
            null -> mods[vk] = false
            false -> mods[vk] = true
            true -> mods.remove(vk)
        }
        onModsChanged?.invoke(modifiers())
    }

    private fun releaseOneShot() {
        val before = mods.size
        mods.entries.removeAll { !it.value }
        if (mods.size != before) onModsChanged?.invoke(modifiers())
    }

    /** Aperta e solta uma tecla, com os modificadores ativos. */
    fun tap(vk: Int) {
        if (!canControl()) return
        val held = mods.keys.toList()
        held.forEach { session.key(it.toUShort(), true) }
        session.key(vk.toUShort(), true)
        session.key(vk.toUShort(), false)
        held.reversed().forEach { session.key(it.toUShort(), false) }
        releaseOneShot()
    }

    /** Combinação fixa (ex.: Ctrl+Shift+Esc), independente dos modificadores ativos. */
    fun combo(vararg vks: Int) {
        if (!canControl()) return
        vks.forEach { session.key(it.toUShort(), true) }
        vks.reversed().forEach { session.key(it.toUShort(), false) }
    }

    fun text(s: CharSequence) {
        if (!canControl()) return
        var i = 0
        val str = s.toString()
        while (i < str.length) {
            val cp = str.codePointAt(i)
            i += Character.charCount(cp)
            val c = if (cp < 0x10000) cp.toChar() else null
            when {
                c == '\n' -> tap(Vk.RETURN)
                c == '\t' -> tap(Vk.TAB)
                mods.isNotEmpty() && c != null && Vk.forAscii(c) != null -> tap(Vk.forAscii(c)!!)
                else -> {
                    session.char(cp.toUInt())
                    releaseOneShot()
                }
            }
        }
    }

    fun backspace(n: Int = 1) = repeat(n) { tap(Vk.BACK) }
    fun forwardDelete(n: Int = 1) = repeat(n) { tap(Vk.DELETE) }

    /** Evento de tecla do IME ou de um teclado físico. */
    fun keyEvent(e: KeyEvent): Boolean {
        if (!canControl()) return false
        val vk = Vk.fromKeyCode(e.keyCode)
        val ctrlOrAlt = e.isCtrlPressed || e.isAltPressed || e.isMetaPressed
        val uc = e.getUnicodeChar(e.metaState)
        if (e.action == KeyEvent.ACTION_DOWN) {
            // Caractere imprimível sem Ctrl/Alt: manda o caractere (independe do layout).
            if (!ctrlOrAlt && uc != 0 && e.keyCode != KeyEvent.KEYCODE_ENTER && e.keyCode != KeyEvent.KEYCODE_TAB &&
                e.keyCode != KeyEvent.KEYCODE_SPACE
            ) {
                text(String(Character.toChars(uc)))
                return true
            }
            if (e.keyCode == KeyEvent.KEYCODE_SPACE && !ctrlOrAlt) {
                text(" ")
                return true
            }
            vk ?: return false
            if (mods.isNotEmpty()) {
                tap(vk)
            } else {
                session.key(vk.toUShort(), true)
            }
            return true
        }
        if (e.action == KeyEvent.ACTION_UP) {
            if (!ctrlOrAlt && uc != 0 && e.keyCode != KeyEvent.KEYCODE_ENTER && e.keyCode != KeyEvent.KEYCODE_TAB) return true
            if (e.keyCode == KeyEvent.KEYCODE_SPACE) return true
            vk ?: return false
            session.key(vk.toUShort(), false)
            return true
        }
        return false
    }
}

/**
 * View invisível que recebe o foco e o IME. Usa `VISIBLE_PASSWORD` + sem sugestões
 * para que o teclado virtual mande cada caractere na hora (sem composição/autocorreção).
 * Se o IME ainda assim compor texto, as diferenças viram Backspace + caracteres.
 */
class KeyCaptureView(context: Context, private val kb: () -> RemoteKeyboard?) : View(context) {
    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor() = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN or
            EditorInfo.IME_ACTION_NONE
        return Conn()
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean =
        if (keyCode == KeyEvent.KEYCODE_BACK) super.onKeyDown(keyCode, event) else kb()?.keyEvent(event) ?: false

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean =
        if (keyCode == KeyEvent.KEYCODE_BACK) super.onKeyUp(keyCode, event) else kb()?.keyEvent(event) ?: false

    fun showKeyboard() {
        requestFocus()
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.restartInput(this)
        imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
    }

    fun hideKeyboard() {
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.hideSoftInputFromWindow(windowToken, 0)
    }

    private inner class Conn : BaseInputConnection(this@KeyCaptureView, false) {
        private var composing = ""

        override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
            val t = text.toString()
            applyComposing(t)
            composing = ""
            return true
        }

        override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
            applyComposing(text.toString())
            composing = text.toString()
            return true
        }

        override fun finishComposingText(): Boolean {
            composing = ""
            return true
        }

        /** Envia só a diferença entre o texto em composição anterior e o novo. */
        private fun applyComposing(new: String) {
            val k = kb() ?: return
            val old = composing
            var p = 0
            while (p < old.length && p < new.length && old[p] == new[p]) p++
            val removed = old.codePointCount(p, old.length)
            if (removed > 0) k.backspace(removed)
            if (p < new.length) k.text(new.substring(p))
        }

        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
            val k = kb() ?: return true
            if (composing.isNotEmpty()) {
                composing = ""
            }
            if (beforeLength > 0) k.backspace(beforeLength)
            if (afterLength > 0) k.forwardDelete(afterLength)
            return true
        }

        override fun sendKeyEvent(event: KeyEvent): Boolean {
            kb()?.keyEvent(event)
            return true
        }
    }
}

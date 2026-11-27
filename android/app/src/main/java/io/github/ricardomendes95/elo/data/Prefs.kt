package io.github.ricardomendes95.elo.data

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject

/** Um endereço salvo pelo usuário. */
data class Favorite(
    val name: String,
    val address: String,
    val lan: Boolean,
    val password: String = "",
    /** Último node id (endereço P2P) visto para este endereço; detecta troca de identidade. */
    val lastNode: String = "",
)

/** Um acesso recente (último conectado primeiro). */
data class Recent(
    val name: String,
    val address: String,
    val lan: Boolean,
    val password: String = "",
    val lastNode: String = "",
)

/** Preferências do app (SharedPreferences + JSON), expostas como StateFlows. */
object Prefs {
    private lateinit var sp: SharedPreferences

    private val _favorites = MutableStateFlow<List<Favorite>>(emptyList())
    val favorites: StateFlow<List<Favorite>> = _favorites.asStateFlow()

    private val _recents = MutableStateFlow<List<Recent>>(emptyList())
    val recents: StateFlow<List<Recent>> = _recents.asStateFlow()

    private val _deviceName = MutableStateFlow("")
    /** Vazio = usar o nome padrão do aparelho. */
    val deviceName: StateFlow<String> = _deviceName.asStateFlow()

    private val _directory = MutableStateFlow("")
    val directory: StateFlow<String> = _directory.asStateFlow()

    private val _theme = MutableStateFlow("dark")
    /** "dark", "light" ou "system". */
    val theme: StateFlow<String> = _theme.asStateFlow()

    private val _touchMode = MutableStateFlow("direct")
    /** "direct" ou "touchpad". */
    val touchMode: StateFlow<String> = _touchMode.asStateFlow()

    private val _audioOnStart = MutableStateFlow(true)
    val audioOnStart: StateFlow<Boolean> = _audioOnStart.asStateFlow()

    private val _keepScreenOn = MutableStateFlow(true)
    val keepScreenOn: StateFlow<Boolean> = _keepScreenOn.asStateFlow()

    private val _hostPassword = MutableStateFlow("")
    /** Senha opcional para quem se conecta a este celular (vazio = pedir aprovação). */
    val hostPassword: StateFlow<String> = _hostPassword.asStateFlow()

    private val _shortId = MutableStateFlow("")
    /** Último ID curto registrado (cache para mostrar offline). */
    val shortId: StateFlow<String> = _shortId.asStateFlow()

    fun setShortId(v: String) { _shortId.value = v; sp.edit().putString("shortId", v).apply() }

    /** Grava o node id visto para [address] em favoritos e recentes. */
    fun setLastNode(address: String, lan: Boolean, node: String) {
        val addr = address.trim()
        if (_favorites.value.any { it.address == addr && it.lan == lan && it.lastNode != node })
            setFavorites(_favorites.value.map { if (it.address == addr && it.lan == lan) it.copy(lastNode = node) else it })
        if (_recents.value.any { it.address == addr && it.lan == lan && it.lastNode != node })
            setRecents(_recents.value.map { if (it.address == addr && it.lan == lan) it.copy(lastNode = node) else it })
    }

    /** Último node id conhecido para [address] (favoritos primeiro, depois recentes). */
    fun lastNode(address: String, lan: Boolean): String {
        val addr = address.trim()
        return _favorites.value.firstOrNull { it.address == addr && it.lan == lan && it.lastNode.isNotEmpty() }?.lastNode
            ?: _recents.value.firstOrNull { it.address == addr && it.lan == lan && it.lastNode.isNotEmpty() }?.lastNode
            ?: ""
    }

    fun init(context: Context) {
        if (::sp.isInitialized) return
        sp = context.applicationContext.getSharedPreferences("elo", Context.MODE_PRIVATE)
        _favorites.value = parseFavorites(sp.getString("favorites", null))
        _recents.value = parseRecents(sp.getString("recents", null))
        _deviceName.value = sp.getString("deviceName", "") ?: ""
        _directory.value = sp.getString("directory", "") ?: ""
        _shortId.value = sp.getString("shortId", "") ?: ""
        _theme.value = sp.getString("theme", "dark") ?: "dark"
        _touchMode.value = sp.getString("touchMode", "direct") ?: "direct"
        _audioOnStart.value = sp.getBoolean("audioOnStart", true)
        _keepScreenOn.value = sp.getBoolean("keepScreenOn", true)
        _hostPassword.value = sp.getString("hostPassword", "") ?: ""
    }

    fun setFavorites(list: List<Favorite>) {
        _favorites.value = list
        sp.edit().putString("favorites", JSONArray().apply {
            list.forEach {
                put(JSONObject().put("name", it.name).put("address", it.address)
                    .put("lan", it.lan).put("password", it.password).put("lastNode", it.lastNode))
            }
        }.toString()).apply()
    }

    /**
     * Adiciona um favorito, evitando duplicar por `address`+`lan`.
     * Retorna `true` se adicionou; `false` se já existia um equivalente.
     */
    fun addFavorite(name: String, address: String, lan: Boolean, password: String): Boolean {
        val addr = address.trim()
        if (_favorites.value.any { it.address == addr && it.lan == lan }) return false
        setFavorites(_favorites.value + Favorite(name.trim().ifBlank { addr }, addr, lan, password))
        return true
    }

    fun setRecents(list: List<Recent>) {
        _recents.value = list
        sp.edit().putString("recents", JSONArray().apply {
            list.forEach {
                put(JSONObject().put("name", it.name).put("address", it.address)
                    .put("lan", it.lan).put("password", it.password).put("lastNode", it.lastNode))
            }
        }.toString()).apply()
    }

    /**
     * Move o acesso para o topo dos recentes (máx. 8), gravando nome e senha.
     * Preserva um nome personalizado (renomeado pelo usuário) e a senha já salva
     * de uma entrada existente: uma reconexão não apaga o que o usuário definiu.
     */
    fun addRecent(name: String, address: String, lan: Boolean, password: String) {
        val addr = address.trim()
        if (addr.isEmpty()) return
        val existing = _recents.value.firstOrNull { it.address == addr && it.lan == lan }
        val newName = name.trim()
        val finalName = when {
            // Nome novo e explícito (diferente do endereço) vence.
            newName.isNotEmpty() && newName != addr -> newName
            // Senão, mantém o nome personalizado que já estava salvo.
            existing != null && existing.name != addr -> existing.name
            else -> newName.ifBlank { addr }
        }
        // Mantém a senha salva quando a reconexão não trouxe uma nova.
        val finalPassword = password.ifBlank { existing?.password ?: "" }
        val entry = Recent(finalName, addr, lan, finalPassword, existing?.lastNode ?: "")
        val list = (listOf(entry) + _recents.value.filter { !(it.address == addr && it.lan == lan) }).take(8)
        setRecents(list)
    }

    fun setDeviceName(v: String) { _deviceName.value = v; sp.edit().putString("deviceName", v).apply() }
    fun setDirectory(v: String) { _directory.value = v.trim(); sp.edit().putString("directory", v.trim()).apply() }
    fun setTheme(v: String) { _theme.value = v; sp.edit().putString("theme", v).apply() }
    fun setTouchMode(v: String) { _touchMode.value = v; sp.edit().putString("touchMode", v).apply() }
    fun setAudioOnStart(v: Boolean) { _audioOnStart.value = v; sp.edit().putBoolean("audioOnStart", v).apply() }
    fun setHostPassword(v: String) { _hostPassword.value = v; sp.edit().putString("hostPassword", v).apply() }
    fun setKeepScreenOn(v: Boolean) { _keepScreenOn.value = v; sp.edit().putBoolean("keepScreenOn", v).apply() }

    private fun parseFavorites(s: String?): List<Favorite> = runCatching {
        val a = JSONArray(s ?: return emptyList())
        (0 until a.length()).map {
            val o = a.getJSONObject(it)
            Favorite(o.getString("name"), o.getString("address"), o.optBoolean("lan"), o.optString("password"), o.optString("lastNode"))
        }
    }.getOrDefault(emptyList())

    /** Lê os recentes, tolerando o formato antigo (lista de strings = só endereço). */
    private fun parseRecents(s: String?): List<Recent> = runCatching {
        val a = JSONArray(s ?: return emptyList())
        (0 until a.length()).map { i ->
            when (val v = a.get(i)) {
                is JSONObject -> Recent(
                    v.optString("name").ifBlank { v.getString("address") },
                    v.getString("address"), v.optBoolean("lan"), v.optString("password"), v.optString("lastNode"),
                )
                else -> {
                    val addr = v.toString()
                    Recent(addr, addr, looksLikeIp(addr), "")
                }
            }
        }
    }.getOrDefault(emptyList())

    /** IPv4 (porta opcional) → inferir "rede local" ao migrar recentes antigos. */
    private fun looksLikeIp(s: String) = Regex("""^\d{1,3}(\.\d{1,3}){3}(:\d+)?$""").matches(s.trim())
}

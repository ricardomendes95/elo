package io.github.ricardomendes95.elo.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.collectAsState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import io.github.ricardomendes95.elo.ui.theme.EloTheme

/** Cartão padrão do Elo (cantos de 12dp, borda sutil). */
@Composable
fun EloCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val c = EloTheme.colors
    Card(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = c.surface, contentColor = c.text),
        border = BorderStroke(1.dp, c.border),
    ) { Column(Modifier.padding(16.dp), content = content) }
}

/** Campo de texto com tema do Elo. */
@Composable
fun EloField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    keyboardType: KeyboardType = KeyboardType.Text,
    singleLine: Boolean = true,
) {
    OutlinedTextField(
        value = value, onValueChange = onValueChange, label = { Text(label) },
        modifier = modifier.fillMaxWidth(), singleLine = singleLine,
        keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
        shape = RoundedCornerShape(12.dp),
    )
}

/** Campo de senha com mostrar/ocultar. */
@Composable
fun PasswordField(value: String, onValueChange: (String) -> Unit, label: String, modifier: Modifier = Modifier) {
    var show by remember { mutableStateOf(false) }
    OutlinedTextField(
        value = value, onValueChange = onValueChange, label = { Text(label) },
        modifier = modifier.fillMaxWidth(), singleLine = true,
        visualTransformation = if (show) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
        trailingIcon = {
            IconButton(onClick = { show = !show }) {
                Icon(
                    if (show) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                    contentDescription = if (show) "Ocultar senha" else "Mostrar senha",
                )
            }
        },
        shape = RoundedCornerShape(12.dp),
    )
}

/** Botão principal (teal, largura cheia). */
@Composable
fun PrimaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    val c = EloTheme.colors
    Button(
        onClick = onClick, enabled = enabled, modifier = modifier.fillMaxWidth().height(48.dp),
        shape = RoundedCornerShape(12.dp),
        colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
    ) { Text(text) }
}

/** Reconhece IPv4 (com porta opcional), usado para inferir "rede local" nos recentes. */
fun looksLikeIp(s: String) = Regex("""^\d{1,3}(\.\d{1,3}){3}(:\d+)?$""").matches(s.trim())

/** Diálogos globais da conexão: 2FA (sempre) e, se [full], também "conectando" e erro. */
@Composable
fun ConnectDialogs(full: Boolean) {
    val state by Connector.state.collectAsState()
    when (val s = state) {
        ConnState.NeedTwoFa -> TwoFaDialog()
        ConnState.IdentityChanged -> AlertDialog(
            onDismissRequest = { Connector.cancel() },
            title = { Text("Identidade alterada") },
            text = { Text("A identidade deste aparelho mudou (reinstalação?). Conectar mesmo assim?") },
            confirmButton = { TextButton(onClick = { Connector.confirmIdentity() }) { Text("Conectar") } },
            dismissButton = { TextButton(onClick = { Connector.cancel() }) { Text("Cancelar") } },
        )
        is ConnState.Connecting -> if (full) {
            AlertDialog(
                onDismissRequest = {},
                title = { Text("Conectando a ${s.label}") },
                text = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.size(12.dp))
                        Text("Conectando… se pediu aprovação, aguarde o outro lado aceitar.")
                    }
                },
                confirmButton = { TextButton(onClick = { Connector.cancel() }) { Text("Cancelar") } },
            )
        }
        is ConnState.Error -> if (full) {
            AlertDialog(
                onDismissRequest = { Connector.dismiss() },
                title = { Text("Não foi possível conectar") },
                text = { Text(s.message, color = EloTheme.colors.danger) },
                confirmButton = { TextButton(onClick = { Connector.dismiss() }) { Text("OK") } },
            )
        }
        ConnState.Idle -> {}
    }
}

@Composable
private fun TwoFaDialog() {
    var code by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = { Connector.cancel() },
        title = { Text("Verificação em duas etapas") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Digite o código de 6 dígitos do aplicativo autenticador.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                EloField(code, { code = it.filter(Char::isDigit).take(6) }, "Código", keyboardType = KeyboardType.NumberPassword)
            }
        },
        confirmButton = {
            TextButton(onClick = { Connector.submitTotp(code) }, enabled = code.length == 6) { Text("Confirmar") }
        },
        dismissButton = { TextButton(onClick = { Connector.cancel() }) { Text("Cancelar") } },
    )
}

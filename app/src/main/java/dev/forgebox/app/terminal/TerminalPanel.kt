package dev.forgebox.app.terminal

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
fun TerminalPanel(
    state: TerminalUiState,
    onWrite: (String) -> Unit,
    onWriteBytes: (ByteArray) -> Unit,
    onSignal: (Int) -> Unit,
    onStop: () -> Unit,
    onCloseSession: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val focusRequester = remember { FocusRequester() }
    var input by remember { mutableStateOf("") }
    val measurer = rememberTextMeasurer()

    Column(modifier.background(Color(0xFF111111))) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("sh", color = Color.White, style = MaterialTheme.typography.labelLarge)
            Text(state.cwdLabel, color = Color(0xFFBBBBBB), style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
            TextButton(onClick = {}, enabled = false) { Text("+ M7") }
            TextButton(onClick = onStop) { Text("Stop") }
            TextButton(onClick = onCloseSession) { Text("End") }
        }
        if (state.hintShown) {
            Text(
                "M6 terminal: real /system/bin/sh + toybox. No python/rustc/node/gcc, no apt, no PRoot/distro, no multi-session, no persistence across app kill.",
                color = Color(0xFFCCAA66),
                style = MaterialTheme.typography.labelSmall,
                modifier = Modifier.padding(horizontal = 8.dp),
            )
        }
        Canvas(
            Modifier.fillMaxWidth().weight(1f).pointerInput(Unit) {
                detectTapGestures(onTap = { focusRequester.requestFocus() })
            }
        ) {
            drawRect(Color(0xFF111111))
            val style = TextStyle(color = Color(0xFFDDDDDD), fontFamily = FontFamily.Monospace, fontSize = 13.sp)
            val lineHeight = 18.dp.toPx()
            state.render.lines.forEachIndexed { i, line ->
                val layout = measurer.measure(line.ifEmpty { " " }, style)
                drawText(layout, topLeft = Offset(4f, i * lineHeight))
            }
            val cx = 4f + state.render.cursorCol * 8.dp.toPx() * 0.6f
            val cy = state.render.cursorRow * lineHeight
            drawRect(Color(0xAAFFFFFF), topLeft = Offset(cx, cy), size = androidx.compose.ui.geometry.Size(8f, lineHeight))
        }

        Row(Modifier.fillMaxWidth().padding(4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            listOf("ESC" to byteArrayOf(27), "TAB" to byteArrayOf(9), "CTRL+C" to byteArrayOf(3), "CTRL+D" to byteArrayOf(4), "CTRL+Z" to byteArrayOf(26)).forEach { (label, bytes) ->
                TextButton(onClick = { onWriteBytes(bytes) }) { Text(label, color = Color.White) }
            }
        }

        BasicTextField(
            value = input,
            onValueChange = { new ->
                if (new.length > input.length) {
                    val added = new.substring(input.length)
                    onWrite(added)
                } else if (new.length < input.length) {
                    onWriteBytes(byteArrayOf(0x7f))
                }
                input = ""
            },
            modifier = Modifier.fillMaxWidth().height(1.dp).focusRequester(focusRequester),
            textStyle = TextStyle(color = Color.Transparent),
            cursorBrush = androidx.compose.ui.graphics.SolidColor(Color.Transparent),
        )
    }
}
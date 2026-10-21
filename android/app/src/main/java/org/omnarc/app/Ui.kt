package org.omnarc.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import omnarc.design.OmnarcTokens

val palette = OmnarcTokens.dark

@Composable
fun OmnarcTheme(content: @Composable () -> Unit) = MaterialTheme(
    colorScheme = darkColorScheme(
        background = palette.bg, surface = palette.surface1, surfaceVariant = palette.surface2,
        primary = palette.turquoise, onPrimary = palette.bg, onBackground = palette.text1,
        onSurface = palette.text1, onSurfaceVariant = palette.text2, outline = palette.line2,
    ),
    content = content,
)

/**
 * The Ring: every arc is a device, as long as its share of the cluster's memory. This phone's arc is the
 * turquoise one; devices that are offline stay as a faint arc, so a leaving device is seen, not hidden.
 */
@Composable
fun Ring(members: List<Member>, size: Dp, stroke: Dp = 14.dp, center: @Composable () -> Unit = {}) {
    Box(Modifier.size(size), contentAlignment = Alignment.Center) {
        Canvas(Modifier.fillMaxSize().padding(stroke / 2)) {
            val width = stroke.toPx()
            val arc = Size(this.size.width, this.size.height)
            val total = members.sumOf { it.memoryBytes.coerceAtLeast(1) }.toFloat()
            if (members.isEmpty()) {
                drawArc(palette.line2, 0f, 360f, false, style = Stroke(width), size = arc, topLeft = Offset.Zero)
                return@Canvas
            }
            val gap = if (members.size > 1) 4f else 0f
            var start = -90f
            for (member in members) {
                val sweep = 360f * member.memoryBytes.coerceAtLeast(1) / total
                val color = when {
                    member.self -> palette.turquoise
                    member.online -> palette.text3
                    else -> palette.line2
                }
                drawArc(color, start + gap / 2, (sweep - gap).coerceAtLeast(1f), false,
                    style = Stroke(width, cap = StrokeCap.Round), size = arc, topLeft = Offset.Zero)
                start += sweep
            }
        }
        center()
    }
}

@Composable
fun Chip(text: String, active: Boolean = false, dot: Color? = null) {
    Row(
        Modifier.clip(RoundedCornerShape(999.dp))
            .background(if (active) palette.turquoise.copy(alpha = .12f) else palette.surface2)
            .border(1.dp, if (active) palette.turquoise.copy(alpha = .35f) else palette.line, RoundedCornerShape(999.dp))
            .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // a status dot drawn as a shape: glyphs like ● depend on the font and once broke a visual test
        if (dot != null) Box(Modifier.size(7.dp).clip(CircleShape).background(dot))
        if (dot != null) Spacer(Modifier.width(6.dp))
        Text(text, color = if (active) palette.text1 else palette.text2, fontSize = OmnarcTokens.FontSize.small)
    }
}

@Composable
fun Stat(value: String, label: String, color: Color = palette.text1, modifier: Modifier = Modifier) {
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Text(value, color = color, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Medium, fontSize = OmnarcTokens.FontSize.body)
        Text(label, color = palette.text3, fontSize = OmnarcTokens.FontSize.micro)
    }
}

@Composable
fun Card(content: @Composable ColumnScope.() -> Unit) = Column(
    Modifier.fillMaxWidth().clip(RoundedCornerShape(OmnarcTokens.Radius.medium)).background(palette.surface1)
        .border(1.dp, palette.line, RoundedCornerShape(OmnarcTokens.Radius.medium)).padding(16.dp),
    verticalArrangement = Arrangement.spacedBy(10.dp),
    content = content,
)


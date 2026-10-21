package org.omnarc.app

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Send
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** The phone app: the node runs in [NodeService]; these screens show and steer it (PLAN §9.7, SPEC AN-01–AN-07). */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 0)
        startForegroundService(Intent(this, NodeService::class.java))
        setContent { OmnarcTheme { App() } }
    }
}

private enum class Tab(val label: String) { Contribute("Contribute"), Cluster("Cluster"), Chat("Chat"), Settings("Settings") }

@Composable
private fun App() {
    val context = LocalContext.current
    var tab by remember { mutableStateOf(Tab.Contribute) }
    var cluster by remember { mutableStateOf<ClusterView?>(null) }
    var device by remember { mutableStateOf(readDevice(context)) }
    LaunchedEffect(Unit) {
        while (true) {
            cluster = withContext(Dispatchers.IO) { fetchCluster() }
            device = readDevice(context)
            delay(3000)
        }
    }
    Scaffold(
        containerColor = palette.bg,
        bottomBar = {
            NavigationBar(containerColor = palette.surface1) {
                for (t in Tab.entries) NavigationBarItem(
                    selected = tab == t, onClick = { tab = t }, label = { Text(t.label) },
                    icon = { TabIcon(t, if (tab == t) palette.turquoise else palette.text3) },
                    colors = NavigationBarItemDefaults.colors(indicatorColor = palette.surface3, selectedTextColor = palette.text1, unselectedTextColor = palette.text3),
                )
            }
        },
    ) { inner ->
        Box(Modifier.padding(inner)) {
            when (tab) {
                Tab.Contribute -> ContributeScreen(cluster, device)
                Tab.Cluster -> ClusterScreen(cluster)
                Tab.Chat -> ChatScreen(cluster)
                Tab.Settings -> SettingsScreen(device)
            }
        }
    }
}

@Composable
private fun TabIcon(tab: Tab, color: Color) = when (tab) {
    Tab.Contribute -> Icon(Icons.Default.Share, null, tint = color)
    Tab.Chat -> Icon(Icons.Default.Send, null, tint = color)
    Tab.Settings -> Icon(Icons.Default.Settings, null, tint = color)
    // the Ring is the brand: drawn, not borrowed from an icon set
    Tab.Cluster -> Canvas(Modifier.size(22.dp)) { drawCircle(color, radius = size.minDimension / 2.6f, style = Stroke(2.2.dp.toPx())) }
}

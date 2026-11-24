package io.github.ricardomendes95.elo.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Star
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import io.github.ricardomendes95.elo.core.EloCore
import io.github.ricardomendes95.elo.ui.favorites.FavoritesScreen
import io.github.ricardomendes95.elo.ui.home.HomeScreen
import io.github.ricardomendes95.elo.ui.home.HostApprovalDialog
import io.github.ricardomendes95.elo.ui.session.SessionScreen
import io.github.ricardomendes95.elo.ui.settings.SettingsScreen
import io.github.ricardomendes95.elo.ui.theme.EloTheme

private const val ROUTE_SESSION = "session"

private data class Tab(val route: String, val label: String, val icon: ImageVector)

private val tabs = listOf(
    Tab("home", "Início", Icons.Outlined.Home),
    Tab("favorites", "Favoritos", Icons.Outlined.Star),
    Tab("settings", "Configurações", Icons.Outlined.Settings),
)

@Composable
fun EloApp() {
    val nav = rememberNavController()
    val entry by nav.currentBackStackEntryAsState()
    val route = entry?.destination?.route
    val active by EloCore.active.collectAsState()
    val c = EloTheme.colors

    // Sessão iniciada -> tela de sessão; sessão encerrada -> volta.
    LaunchedEffect(active) {
        val onSession = nav.currentDestination?.route == ROUTE_SESSION
        if (active != null && !onSession) nav.navigate(ROUTE_SESSION)
        else if (active == null && onSession) nav.popBackStack()
    }

    HostApprovalDialog()

    Scaffold(
        containerColor = c.bg,
        bottomBar = {
            if (route != ROUTE_SESSION) {
                NavigationBar(containerColor = c.bar, contentColor = c.text) {
                    tabs.forEach { t ->
                        NavigationBarItem(
                            selected = route == t.route,
                            onClick = {
                                nav.navigate(t.route) {
                                    popUpTo(nav.graph.findStartDestination().id) { saveState = true }
                                    launchSingleTop = true
                                    restoreState = true
                                }
                            },
                            icon = { Icon(t.icon, contentDescription = t.label) },
                            label = { Text(t.label) },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor = c.onAccent, selectedTextColor = c.accent,
                                indicatorColor = c.accent, unselectedIconColor = c.muted,
                                unselectedTextColor = c.muted,
                            ),
                        )
                    }
                }
            }
        },
    ) { pad ->
        NavHost(
            navController = nav,
            startDestination = "home",
            modifier = if (route == ROUTE_SESSION) Modifier else Modifier.padding(pad),
        ) {
            composable("home") { HomeScreen() }
            composable("favorites") { FavoritesScreen() }
            composable("settings") { SettingsScreen() }
            composable(ROUTE_SESSION) {
                val a = active
                if (a != null) {
                    SessionScreen(a) {
                        a.close()
                        if (nav.currentDestination?.route == ROUTE_SESSION) nav.popBackStack()
                    }
                }
            }
        }
    }
}

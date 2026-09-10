package app.yasmine.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.QueueMusic
import androidx.compose.material.icons.filled.LibraryMusic
import androidx.compose.material.icons.filled.Smartphone
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import app.yasmine.playback.rememberPlayerConnection
import app.yasmine.ui.library.LibraryScreen
import app.yasmine.ui.pair.PairScreen
import app.yasmine.ui.player.NowPlayingBar
import app.yasmine.ui.player.NowPlayingScreen
import app.yasmine.ui.playlists.PlaylistsScreen

private const val PLAYER_ROUTE = "player"

private enum class Tab(val route: String, val label: String) {
    Library("library", "Library"),
    Playlists("playlists", "Playlists"),
    Sync("pair", "Sync"),
}

@Composable
fun YasmineNav() {
    val nav = rememberNavController()
    val player = rememberPlayerConnection()
    val backStack by nav.currentBackStackEntryAsState()
    val current = backStack?.destination
    val scheme = MaterialTheme.colorScheme
    val onPlayer = current?.route == PLAYER_ROUTE

    val openPlayer: () -> Unit = {
        nav.navigate(PLAYER_ROUTE) { launchSingleTop = true }
    }

    Scaffold(
        containerColor = scheme.background,
        bottomBar = {
            if (!onPlayer) Column {
                NowPlayingBar(player, onOpen = openPlayer)
                NavigationBar(containerColor = scheme.surface, tonalElevation = 0.dp) {
                    Tab.entries.forEach { tab ->
                        val selected = current?.hierarchy?.any { it.route == tab.route } == true
                        NavigationBarItem(
                            selected = selected,
                            onClick = {
                                nav.navigate(tab.route) {
                                    popUpTo(nav.graph.findStartDestination().id) { saveState = true }
                                    launchSingleTop = true
                                    restoreState = true
                                }
                            },
                            icon = {
                                Icon(
                                    when (tab) {
                                        Tab.Library -> Icons.Filled.LibraryMusic
                                        Tab.Playlists -> Icons.AutoMirrored.Filled.QueueMusic
                                        Tab.Sync -> Icons.Filled.Smartphone
                                    },
                                    contentDescription = tab.label,
                                )
                            },
                            label = { Text(tab.label) },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor = scheme.onPrimary,
                                selectedTextColor = scheme.onSurface,
                                indicatorColor = scheme.primary,
                                unselectedIconColor = scheme.onSurfaceVariant,
                                unselectedTextColor = scheme.onSurfaceVariant,
                            ),
                        )
                    }
                }
            }
        },
    ) { padding ->
        NavHost(
            navController = nav,
            startDestination = Tab.Library.route,
            modifier = Modifier
                .fillMaxSize()
                .background(scheme.background)
                .padding(padding),
        ) {
            composable(Tab.Library.route) { LibraryScreen(player, onOpenPlayer = openPlayer) }
            composable(Tab.Playlists.route) { PlaylistsScreen(player, onOpenPlayer = openPlayer) }
            composable(Tab.Sync.route) { PairScreen() }
            composable(PLAYER_ROUTE) {
                NowPlayingScreen(player, onClose = { nav.popBackStack() })
            }
        }
    }
}

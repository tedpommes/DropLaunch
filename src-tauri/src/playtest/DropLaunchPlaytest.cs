// DropLaunch: bug reports from Rust's chat for DropLaunch's playtest page.
//
// DropLaunch (https://droplaunch.gg) puts this file in the server's plugins folder while its
// playtest tools are on, and replaces or removes it: changes to this copy don't last.
//
//   /bug <what's wrong>        (chat, every player) a report with the player's position, view
//                              and what they look at, printed to the log for DropLaunch.
//   /droplaunch kit [name]     (chat, admins) what the player wears, holds in the belt and the
//                              inventory, printed to the log: DropLaunch adds it to its kits.
//   Whitelist: when DropLaunch's whitelist is on (data/DropLaunch/whitelist.json, written by
//   DropLaunch), only the players in it can join.
//   droplaunch.players         (server console / RCON only) who's where, as JSON.
//   droplaunch.tp <steamid> <x> <y> <z> [yaw]   (console / RCON) teleport a player there.
//   droplaunch.tpto <steamid|*> <x> <z>         (console / RCON) teleport a player, or everyone,
//                              to the ground (or water) at that spot of the map.
//   droplaunch.tell <steamid> <text>            (console / RCON) a chat message to one player.
//   droplaunch.monuments       (console / RCON) the monuments the in-game map names, with where they are.
//   droplaunch.items           (console / RCON) every item: short name, English name, category, stack size.
//   droplaunch.givekit <steamid|*> <slots>      (console / RCON) a kit into players' wear, belt and
//                              inventory: slots are  w:0:hat.wolf:1;b:0:rifle.ak:1;m:3:ammo.rifle:128
//                              (container w/b/m, slot, short name, amount).
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using Oxide.Core;
using UnityEngine;

namespace Oxide.Plugins
{
    [Info("DropLaunch", "DropLaunch", "1.4.0")]
    [Description("Testers type /bug in chat and the report shows up on DropLaunch's map, where they stood.")]
    public class DropLaunchPlaytest : RustPlugin
    {
        private const string Marker = "DLBUG1 ";
        private const string KitMarker = "DLKITSAVE1 ";
        private const string WhitelistFile = "DropLaunch/whitelist";
        private const float Cooldown = 3f;
        private const int MaxText = 300;
        private static readonly CultureInfo Inv = CultureInfo.InvariantCulture;
        private readonly Dictionary<string, float> lastReport = new Dictionary<string, float>();
        private string gridJson;

        // ------------------------------------------------------------------ chat

        [ChatCommand("bug")]
        private void BugCommand(BasePlayer player, string command, string[] args)
        {
            if (player == null) return;
            var text = string.Join(" ", args ?? new string[0]).Trim();
            if (text.Length == 0)
            {
                player.ChatMessage("<color=#7a95e3>DropLaunch</color>: type <color=#e6b450>/bug</color> and what's wrong, like <color=#e6b450>/bug floating rock</color>. Where you stand and look goes with it.");
                return;
            }
            if (text.Length > MaxText) text = text.Substring(0, MaxText);
            float last;
            var now = Time.realtimeSinceStartup;
            if (lastReport.TryGetValue(player.UserIDString, out last) && now - last < Cooldown)
            {
                player.ChatMessage("<color=#7a95e3>DropLaunch</color>: one moment, then send the next one.");
                return;
            }
            lastReport[player.UserIDString] = now;

            var pos = player.transform.position;
            var angles = player.eyes.rotation.eulerAngles;
            var grid = MapHelper.PositionToString(pos);
            var sb = new StringBuilder(512);
            sb.Append('{');
            Field(sb, "id", DateTime.UtcNow.Ticks.ToString(Inv) + "-" + player.UserIDString);
            Field(sb, "steamId", player.UserIDString);
            Field(sb, "name", player.displayName ?? "");
            Field(sb, "text", text);
            Field(sb, "grid", grid ?? "");
            Num(sb, "x", pos.x);
            Num(sb, "y", pos.y);
            Num(sb, "z", pos.z);
            Num(sb, "yaw", angles.y);
            Num(sb, "pitch", angles.x > 180f ? angles.x - 360f : angles.x);
            Num(sb, "size", World.Size);
            RaycastHit hit;
            if (Physics.Raycast(player.eyes.HeadRay(), out hit, 300f, LookMask(), QueryTriggerInteraction.Ignore))
            {
                Field(sb, "look", Describe(hit) ?? "");
                Num(sb, "lookX", hit.point.x);
                Num(sb, "lookY", hit.point.y);
                Num(sb, "lookZ", hit.point.z);
                Num(sb, "lookDistance", hit.distance);
            }
            sb.Length--; // the last comma
            sb.Append('}');
            Puts(Marker + sb);
            player.ChatMessage("<color=#7a95e3>DropLaunch</color>: bug report sent (" + (grid ?? "?") + ").");
        }

        [ChatCommand("droplaunch")]
        private void DropLaunchCommand(BasePlayer player, string command, string[] args)
        {
            if (player == null) return;
            if (args == null || args.Length == 0 || !string.Equals(args[0], "kit", StringComparison.OrdinalIgnoreCase))
            {
                player.ChatMessage("<color=#7a95e3>DropLaunch</color>: type <color=#e6b450>/droplaunch kit</color> (or <color=#e6b450>/droplaunch kit Raid kit</color>) to save what you wear and carry as a kit in DropLaunch.");
                return;
            }
            if (!player.IsAdmin)
            {
                player.ChatMessage("<color=#7a95e3>DropLaunch</color>: only admins can save kits.");
                return;
            }
            var name = string.Join(" ", args, 1, args.Length - 1).Trim();
            if (name.Length > 60) name = name.Substring(0, 60);
            var sb = new StringBuilder(1024);
            sb.Append('{');
            Field(sb, "id", DateTime.UtcNow.Ticks.ToString(Inv) + "-" + player.UserIDString);
            Field(sb, "name", name);
            Field(sb, "by", player.displayName ?? "");
            var count = 0;
            count += Items(sb, "wear", player.inventory.containerWear);
            sb.Append(',');
            count += Items(sb, "belt", player.inventory.containerBelt);
            sb.Append(',');
            count += Items(sb, "main", player.inventory.containerMain);
            sb.Append('}');
            if (count == 0)
            {
                player.ChatMessage("<color=#7a95e3>DropLaunch</color>: you're not carrying anything to save.");
                return;
            }
            Puts(KitMarker + sb);
            player.ChatMessage("<color=#7a95e3>DropLaunch</color>: kit saved (" + count.ToString(Inv) + " items). It's in DropLaunch's Give items.");
        }

        /// A container's items as "key":[{"item":..,"amount":..,"slot":..}]; how many.
        private static int Items(StringBuilder sb, string key, ItemContainer container)
        {
            sb.Append('"').Append(key).Append("\":[");
            var count = 0;
            if (container != null)
            {
                foreach (var item in container.itemList)
                {
                    if (item == null || item.info == null || item.amount < 1) continue;
                    if (count > 0) sb.Append(',');
                    count++;
                    sb.Append("{\"item\":\"").Append(item.info.shortname).Append("\",\"amount\":").Append(item.amount.ToString(Inv)).Append(",\"slot\":").Append(item.position.ToString(Inv)).Append('}');
                }
            }
            sb.Append(']');
            return count;
        }

        // ------------------------------------------------------------------ whitelist

        private class WhitelistData
        {
            public bool on;
            public List<string> ids = new List<string>();
        }

        /// DropLaunch's whitelist, read at every login (so a change counts right away). Off, or
        /// nobody in it: everyone can join.
        private object CanUserLogin(string name, string id, string ipAddress)
        {
            WhitelistData list = null;
            try
            {
                if (Interface.Oxide.DataFileSystem.ExistsDatafile(WhitelistFile)) list = Interface.Oxide.DataFileSystem.ReadObject<WhitelistData>(WhitelistFile);
            }
            catch (Exception)
            {
                list = null;
            }
            if (list == null || !list.on || list.ids == null || list.ids.Count == 0 || list.ids.Contains(id)) return null;
            Puts("Not on DropLaunch's whitelist, not let in: " + (name ?? "") + " (" + id + ")");
            return "This test server is whitelisted: ask its owner to add you in DropLaunch (Settings, Servers).";
        }

        // ------------------------------------------------------------------ console / RCON (DropLaunch)

        [ConsoleCommand("droplaunch.players")]
        private void PlayersCommand(ConsoleSystem.Arg arg)
        {
            if (arg.Connection != null) return;
            var sb = new StringBuilder(256);
            sb.Append("{\"v\":1,");
            Num(sb, "size", World.Size);
            sb.Append("\"grid\":").Append(Grid()).Append(",\"players\":[");
            var first = true;
            foreach (var p in BasePlayer.activePlayerList)
            {
                if (p == null) continue;
                if (!first) sb.Append(',');
                first = false;
                var pos = p.transform.position;
                sb.Append('{');
                Field(sb, "id", p.UserIDString);
                Field(sb, "name", p.displayName ?? "");
                Num(sb, "x", pos.x);
                Num(sb, "y", pos.y);
                Num(sb, "z", pos.z);
                Num(sb, "yaw", p.eyes.rotation.eulerAngles.y);
                sb.Append("\"sleeping\":").Append(p.IsSleeping() ? "true" : "false").Append(',');
                sb.Append("\"dead\":").Append(p.IsDead() ? "true" : "false");
                sb.Append('}');
            }
            sb.Append("]}");
            arg.ReplyWith(sb.ToString());
        }

        [ConsoleCommand("droplaunch.tp")]
        private void TeleportCommand(ConsoleSystem.Arg arg)
        {
            if (arg.Connection != null) return;
            // GetString/HasArgs rather than arg.Args: newer Rust servers keep arguments as StringView.
            float x, y, z, yaw;
            if (!arg.HasArgs(4) || !Parse(arg.GetString(1), out x) || !Parse(arg.GetString(2), out y) || !Parse(arg.GetString(3), out z))
            {
                arg.ReplyWith("DLTP usage");
                return;
            }
            var player = Find(arg.GetString(0));
            if (player == null)
            {
                arg.ReplyWith("DLTP not-found");
                return;
            }
            if (player.IsDead())
            {
                arg.ReplyWith("DLTP dead");
                return;
            }
            if (player.isMounted) player.EnsureDismounted();
            player.Teleport(new Vector3(x, y, z));
            if (arg.HasArgs(5) && Parse(arg.GetString(4), out yaw)) player.OverrideViewAngles(new Vector3(0f, yaw, 0f));
            arg.ReplyWith("DLTP ok");
        }

        [ConsoleCommand("droplaunch.tpto")]
        private void TeleportToCommand(ConsoleSystem.Arg arg)
        {
            if (arg.Connection != null) return;
            float x, z;
            if (!arg.HasArgs(3) || !Parse(arg.GetString(1), out x) || !Parse(arg.GetString(2), out z))
            {
                arg.ReplyWith("DLTP usage");
                return;
            }
            var who = arg.GetString(0);
            var targets = new List<BasePlayer>();
            foreach (var p in BasePlayer.activePlayerList)
            {
                if (p != null && !p.IsDead() && (who == "*" || p.UserIDString == who)) targets.Add(p);
            }
            if (targets.Count == 0)
            {
                arg.ReplyWith("DLTP not-found");
                return;
            }
            for (var i = 0; i < targets.Count; i++)
            {
                // Everyone side by side around the spot, not inside each other.
                var r = targets.Count > 1 ? 1.5f + 0.3f * i : 0f;
                var px = x + Mathf.Cos(i * 2.4f) * r;
                var pz = z + Mathf.Sin(i * 2.4f) * r;
                var player = targets[i];
                if (player.isMounted) player.EnsureDismounted();
                player.Teleport(new Vector3(px, Ground(px, pz) + 0.25f, pz));
            }
            arg.ReplyWith("DLTP ok " + targets.Count.ToString(Inv));
        }

        [ConsoleCommand("droplaunch.monuments")]
        private void MonumentsCommand(ConsoleSystem.Arg arg)
        {
            if (arg.Connection != null) return;
            var sb = new StringBuilder("{\"monuments\":[");
            var first = true;
            // Unity's own lookup: TerrainPath's list isn't public in every setup (Oxide).
            var list = UnityEngine.Object.FindObjectsOfType<MonumentInfo>();
            if (list != null)
            {
                foreach (var m in list)
                {
                    // The ones the in-game map shows a name for.
                    if (m == null || !m.shouldDisplayOnMap) continue;
                    var name = MonumentName(m);
                    if (name.Length == 0) continue;
                    if (!first) sb.Append(',');
                    first = false;
                    var pos = m.transform.position;
                    sb.Append('{');
                    Field(sb, "name", name);
                    Num(sb, "x", pos.x);
                    Num(sb, "z", pos.z);
                    sb.Length--;
                    sb.Append('}');
                }
            }
            sb.Append("]}");
            arg.ReplyWith(sb.ToString());
        }

        [ConsoleCommand("droplaunch.items")]
        private void ItemsCommand(ConsoleSystem.Arg arg)
        {
            if (arg.Connection != null) return;
            var sb = new StringBuilder(64 * 1024);
            sb.Append("{\"items\":[");
            var first = true;
            foreach (var def in ItemManager.itemList)
            {
                if (def == null || def.hidden || string.IsNullOrEmpty(def.shortname)) continue;
                if (!first) sb.Append(',');
                first = false;
                sb.Append('{');
                Field(sb, "s", def.shortname);
                Field(sb, "n", def.displayName != null && !string.IsNullOrEmpty(def.displayName.english) ? def.displayName.english : def.shortname);
                Field(sb, "c", def.category.ToString());
                Num(sb, "st", Mathf.Max(1, def.stackable));
                sb.Length--;
                sb.Append('}');
            }
            sb.Append("]}");
            arg.ReplyWith(sb.ToString());
        }

        [ConsoleCommand("droplaunch.givekit")]
        private void GiveKitCommand(ConsoleSystem.Arg arg)
        {
            if (arg.Connection != null) return;
            if (!arg.HasArgs(2))
            {
                arg.ReplyWith("DLKIT usage");
                return;
            }
            var who = arg.GetString(0);
            var spec = arg.GetString(1);
            var players = 0;
            var given = 0;
            foreach (var p in BasePlayer.activePlayerList)
            {
                if (p == null || p.IsDead() || (who != "*" && p.UserIDString != who)) continue;
                players++;
                given += GiveKit(p, spec);
            }
            arg.ReplyWith(players == 0 ? "DLKIT not-found" : "DLKIT ok " + players.ToString(Inv) + " " + given.ToString(Inv));
        }

        /// Each entry as one stack of the whole amount (like inventory.giveto, even above Rust's
        /// stack size) into its own slot when that's free, else anywhere in the player's
        /// inventory. How many items were made.
        private static int GiveKit(BasePlayer player, string spec)
        {
            var made = 0;
            foreach (var entry in spec.Split(';'))
            {
                var parts = entry.Split(':');
                int slot, amount;
                if (parts.Length != 4 || !int.TryParse(parts[1], NumberStyles.Integer, Inv, out slot) || !int.TryParse(parts[3], NumberStyles.Integer, Inv, out amount) || amount < 1) continue;
                var def = ItemManager.FindItemDefinition(parts[2]);
                if (def == null) continue;
                var container = parts[0] == "w" ? player.inventory.containerWear : parts[0] == "b" ? player.inventory.containerBelt : player.inventory.containerMain;
                var item = ItemManager.Create(def, amount, 0UL);
                if (item == null) continue;
                made++;
                if (container != null && slot >= 0 && slot < container.capacity && container.GetSlot(slot) == null && item.MoveToContainer(container, slot, false, true)) continue;
                // Rust drops it at their feet when the inventory is full.
                player.GiveItem(item);
            }
            return made;
        }

        /// The in-game map's name for a monument, else its prefab's file name made readable
        /// (custom monuments often have no display name).
        private static string MonumentName(MonumentInfo m)
        {
            if (m.displayPhrase != null && !string.IsNullOrEmpty(m.displayPhrase.english)) return m.displayPhrase.english.Trim();
            var name = m.name ?? "";
            var slash = name.LastIndexOf('/');
            if (slash >= 0) name = name.Substring(slash + 1);
            if (name.EndsWith(".prefab", StringComparison.OrdinalIgnoreCase)) name = name.Substring(0, name.Length - 7);
            return name.Replace('_', ' ').Trim();
        }

        [ConsoleCommand("droplaunch.tell")]
        private void TellCommand(ConsoleSystem.Arg arg)
        {
            if (arg.Connection != null || !arg.HasArgs(2)) return;
            var player = Find(arg.GetString(0));
            if (player == null) return;
            var words = new List<string>();
            for (var i = 1; arg.HasArgs(i + 1); i++) words.Add(arg.GetString(i));
            player.ChatMessage("<color=#7a95e3>DropLaunch</color>: " + string.Join(" ", words.ToArray()));
        }

        // ------------------------------------------------------------------ helpers

        private static BasePlayer Find(string steamId)
        {
            foreach (var p in BasePlayer.activePlayerList)
            {
                if (p != null && p.UserIDString == steamId) return p;
            }
            return null;
        }

        /// The top of whatever is at a spot: the terrain, a rock or building on it, or the water
        /// (a lake's surface, the sea's above the sea bed).
        private static float Ground(float x, float z)
        {
            var spot = new Vector3(x, 0f, z);
            var y = TerrainMeta.HeightMap != null ? TerrainMeta.HeightMap.GetHeight(spot) : 0f;
            RaycastHit hit;
            if (Physics.Raycast(new Vector3(x, 1000f, z), Vector3.down, out hit, 1600f, LookMask(), QueryTriggerInteraction.Ignore)) y = hit.point.y;
            if (TerrainMeta.WaterMap != null) y = Mathf.Max(y, TerrainMeta.WaterMap.GetHeight(spot));
            return Mathf.Max(y, 0f);
        }

        private static int LookMask()
        {
            return LayerMask.GetMask("Terrain", "World", "Construction", "Deployed", "Default", "Tree", "Resource", "Water", "Vehicle_Large", "Vehicle_World", "Clutter");
        }

        /// What a look ends on: an entity's prefab, the map prefab around a collider, or the ground.
        private static string Describe(RaycastHit hit)
        {
            var entity = RaycastHitEx.GetEntity(hit);
            if (entity != null) return entity.ShortPrefabName;
            if (hit.collider == null) return null;
            for (var t = hit.collider.transform; t != null; t = t.parent)
            {
                var name = t.name ?? "";
                if (name.StartsWith("assets/", StringComparison.OrdinalIgnoreCase))
                {
                    var slash = name.LastIndexOf('/');
                    var file = slash >= 0 ? name.Substring(slash + 1) : name;
                    return file.EndsWith(".prefab", StringComparison.OrdinalIgnoreCase) ? file.Substring(0, file.Length - 7) : file;
                }
            }
            var layer = LayerMask.LayerToName(hit.collider.gameObject.layer);
            if (layer == "Terrain") return "terrain";
            if (layer == "Water") return "water";
            return hit.collider.name;
        }

        /// Where Rust's map grid lies (the in-game map's letters and numbers): the size of a cell and
        /// the world position of its top left corner, measured with Rust's own grid names.
        private string Grid()
        {
            if (gridJson != null) return gridJson;
            float half = World.Size / 2f;
            float x1 = Boundary(half, true, 0.25f), x2 = Boundary(half, true, x1 + 0.25f);
            float z1 = Boundary(half, false, 0.25f), z2 = Boundary(half, false, z1 + 0.25f);
            float cell = x2 - x1;
            var sb = new StringBuilder("{");
            Num(sb, "cell", cell);
            Num(sb, "cellZ", z2 - z1);
            Num(sb, "x0", -half + x1 - cell);
            Num(sb, "z0", half - z1 + (z2 - z1));
            Field(sb, "first", MapHelper.PositionToString(new Vector3(-half + 0.5f, 0f, half - 0.5f)) ?? "");
            sb.Length--;
            sb.Append('}');
            gridJson = sb.ToString();
            return gridJson;
        }

        /// How far from the west (or north) edge, after `from`, the grid's column (row) changes.
        private static float Boundary(float half, bool columns, float from)
        {
            Func<float, int> cellAt = d => columns
                ? MapHelper.PositionToGrid(new Vector3(-half + d, 0f, half - 0.5f)).x
                : MapHelper.PositionToGrid(new Vector3(-half + 0.5f, 0f, half - d)).y;
            float lo = from, hi = from + 2000f;
            int start = cellAt(lo);
            if (cellAt(hi) == start) return hi;
            for (int i = 0; i < 40; i++)
            {
                float mid = (lo + hi) / 2f;
                if (cellAt(mid) == start) lo = mid; else hi = mid;
            }
            return hi;
        }

        private static bool Parse(string text, out float value)
        {
            return float.TryParse(text, NumberStyles.Float, Inv, out value);
        }

        private static void Field(StringBuilder sb, string key, string value)
        {
            sb.Append('"').Append(key).Append("\":\"");
            foreach (var c in value)
            {
                if (c == '"') sb.Append("\\\"");
                else if (c == '\\') sb.Append("\\\\");
                else if (c < ' ') sb.Append("\\u").Append(((int)c).ToString("x4", Inv));
                else sb.Append(c);
            }
            sb.Append("\",");
        }

        private static void Num(StringBuilder sb, string key, float value)
        {
            sb.Append('"').Append(key).Append("\":").Append(float.IsNaN(value) || float.IsInfinity(value) ? "0" : value.ToString("0.###", Inv)).Append(',');
        }
    }
}

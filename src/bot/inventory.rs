//! Inventory + window interaction: `click_window` (the stateful 1.21
//! `container_click` with predicted changed slots), plus the transfer helpers
//! (`put_away`, `put_selected_item_range`, `move_slot_item`), equip, toss, and
//! container open/close. Port of typecraft's `bot/inventory.ts`.

use std::time::Duration;
use std::time::Instant;

use crate::item::{items_equal, to_notch, Item};
use crate::protocol::PValue;
use crate::window::{Click, Window};

use super::{Bot, DriveStep, Face};

/// Equipment destination → player-inventory slot index.
fn equip_slot(dest: &str) -> i32 {
    match dest {
        "hand" => 36,
        "off-hand" => 45,
        "head" => 5,
        "torso" => 6,
        "legs" => 7,
        "feet" => 8,
        _ => -1,
    }
}

impl<'a> Bot<'a> {
    /// The active window (open container, else the player inventory).
    fn active_window(&mut self) -> &mut Window {
        if self.current_window.is_some() {
            self.current_window.as_mut().unwrap()
        } else {
            &mut self.inventory
        }
    }

    /// Click a window slot (`mode`/`mouse_button` mirror the Click Window packet),
    /// simulating the result client-side and sending the predicted changed slots.
    pub async fn click_window(&mut self, slot: i32, mouse_button: i32, mode: i32) -> std::io::Result<()> {
        let registry = self.registry;
        self.next_action_id += 1;

        let window = self.active_window();
        let window_id = window.id;
        let state_id = window.state_id;
        let old: Vec<Option<Item>> = window.slots.clone();
        window.accept_click(registry, Click { mode, mouse_button, slot }, 0);

        let mut changed = Vec::new();
        for i in 0..window.slots.len() {
            if !items_equal(old[i].as_ref(), window.slots[i].as_ref(), true, true) {
                changed.push(PValue::compound(vec![
                    ("location", PValue::num(i as f64)),
                    ("item", to_notch(registry, window.slots[i].as_ref())),
                ]));
            }
        }
        let cursor = to_notch(registry, window.selected_item.as_ref());

        self.client
            .write(
                "container_click",
                PValue::compound(vec![
                    ("windowId", PValue::num(window_id as f64)),
                    ("stateId", PValue::num(state_id as f64)),
                    ("slot", PValue::num(slot as f64)),
                    ("mouseButton", PValue::num(mouse_button as f64)),
                    ("mode", PValue::num(mode as f64)),
                    ("changedSlots", PValue::List(changed)),
                    ("cursorItem", cursor),
                ]),
            )
            .await?;

        self.wait_for_inventory_ack(Duration::from_millis(1000)).await
    }

    /// Drive the loop until the server sends a slot/content update (or timeout).
    async fn wait_for_inventory_ack(&mut self, timeout: Duration) -> std::io::Result<()> {
        let rev = self.inv_revision;
        let deadline = Instant::now() + timeout;
        while self.inv_revision == rev && Instant::now() < deadline {
            if matches!(self.drive_tick().await?, DriveStep::Disconnected) {
                return Ok(());
            }
        }
        Ok(())
    }

    fn selected_item(&self) -> Option<&Item> {
        if let Some(w) = &self.current_window {
            w.selected_item.as_ref()
        } else {
            self.inventory.selected_item.as_ref()
        }
    }

    /// Bounds (start, end) of the active window's player-inventory section.
    fn inventory_range(&self) -> (usize, usize) {
        let w = self.current_window.as_ref().unwrap_or(&self.inventory);
        (w.inventory_start, w.inventory_end)
    }

    fn slot_item(&self, i: usize) -> Option<&Item> {
        let w = self.current_window.as_ref().unwrap_or(&self.inventory);
        w.slots.get(i).and_then(|s| s.as_ref())
    }

    /// Put the held (cursor) item away into [start, end), filling partial stacks
    /// first, else an empty slot, else tossing.
    pub async fn put_selected_item_range(
        &mut self,
        start: usize,
        end: usize,
        fallback_slot: i32,
    ) -> std::io::Result<()> {
        while let Some(sel) = self.selected_item().cloned() {
            let mut dest: Option<usize> = None;
            for i in start..end {
                if let Some(item) = self.slot_item(i) {
                    if item.type_id == sel.type_id
                        && item.metadata == sel.metadata
                        && item.count < item.stack_size
                    {
                        dest = Some(i);
                        break;
                    }
                }
            }
            if dest.is_none() {
                for i in start..end {
                    if self.slot_item(i).is_none() {
                        dest = Some(i);
                        break;
                    }
                }
            }
            match dest {
                Some(d) => self.click_window(d as i32, 0, 0).await?,
                None => {
                    if fallback_slot >= 0 {
                        self.click_window(fallback_slot, 0, 0).await?;
                    }
                    self.click_window(-999, 0, 0).await?; // toss
                    break;
                }
            }
        }
        Ok(())
    }

    /// Pick up the item at `slot` and stash it into the inventory section.
    pub async fn put_away(&mut self, slot: i32) -> std::io::Result<()> {
        self.click_window(slot, 0, 0).await?;
        let (start, end) = self.inventory_range();
        self.put_selected_item_range(start, end, slot).await
    }

    /// Move an item from one slot to another (swapping back if needed).
    pub async fn move_slot_item(&mut self, source: i32, dest: i32) -> std::io::Result<()> {
        self.click_window(source, 0, 0).await?;
        self.click_window(dest, 0, 0).await?;
        if self.selected_item().is_some() {
            self.click_window(source, 0, 0).await?;
        }
        Ok(())
    }

    /// Select a hotbar slot (0-8).
    pub async fn set_quick_bar_slot(&mut self, slot: i32) -> std::io::Result<()> {
        self.held_slot = slot;
        self.client.write("set_carried_item", PValue::compound(vec![("slotId", PValue::num(slot as f64))])).await
    }

    /// Equip an item (by type id) to a destination ("hand", "head", …).
    pub async fn equip(&mut self, item_type: i32, destination: &str) -> std::io::Result<()> {
        let dest_slot = equip_slot(destination);
        if dest_slot == -1 {
            return Ok(());
        }
        let found = self
            .inventory
            .slots
            .iter()
            .position(|s| s.as_ref().map(|i| i.type_id) == Some(item_type));
        if let Some(i) = found {
            self.click_window(i as i32, 0, 0).await?;
            self.click_window(dest_slot, 0, 0).await?;
        }
        Ok(())
    }

    /// Drop `count` of an item type from the inventory.
    pub async fn toss(&mut self, item_type: i32, count: i32) -> std::io::Result<()> {
        let mut remaining = count.max(1);
        let mut i = 0;
        while i < self.inventory.slots.len() && remaining > 0 {
            if self.inventory.slots[i].as_ref().map(|it| it.type_id) == Some(item_type) {
                self.click_window(i as i32, 0, 4).await?; // mode 4 = drop
                remaining -= 1;
            }
            i += 1;
        }
        Ok(())
    }

    /// Right-click a block to open its container, returning once a window opens.
    pub async fn open_block(&mut self, x: i32, y: i32, z: i32, face: Face) -> std::io::Result<bool> {
        self.place_block(x, y, z, face).await?; // use_item_on
        let deadline = Instant::now() + Duration::from_millis(5000);
        while self.current_window.is_none() && Instant::now() < deadline {
            if matches!(self.drive_tick().await?, DriveStep::Disconnected) {
                return Ok(false);
            }
        }
        if self.current_window.is_none() {
            return Ok(false);
        }
        // The window opened, but `handle_open_screen` SEEDED its inventory section
        // with our *client-side* (possibly phantom) inventory. The server's
        // authoritative `container_set_content` arrives a beat later and overwrites
        // it with the truth. Wait for that content packet (an inv_revision bump)
        // before returning, so callers (e.g. craft) read REAL ingredient counts and
        // can't place phantom clicks for items the server doesn't actually have.
        let rev = self.inv_revision;
        let content_deadline = Instant::now() + Duration::from_millis(2000);
        while self.inv_revision == rev && Instant::now() < content_deadline {
            if matches!(self.drive_tick().await?, DriveStep::Disconnected) {
                return Ok(false);
            }
        }
        Ok(self.current_window.is_some())
    }

    /// Close the open container, syncing its inventory section back.
    pub async fn close_window(&mut self) -> std::io::Result<()> {
        let id = match &self.current_window {
            Some(w) => w.id,
            None => return Ok(()),
        };
        self.client.write("container_close", PValue::compound(vec![("windowId", PValue::num(id as f64))])).await?;
        self.sync_current_window();
        Ok(())
    }

    /// Total count of an item in the player inventory.
    pub fn item_count(&self, name: &str) -> i32 {
        self.inventory.slots.iter().flatten().filter(|i| i.name == name).map(|i| i.count).sum()
    }

    /// Reflect a just-crafted item locally if the (racy) container sync missed it
    /// — the server crafted it for us, so our inventory should show it too. Adds
    /// to an existing matching stack or the first free inventory slot.
    pub fn ensure_item(&mut self, name: &str, count: i32) {
        if count <= 0 {
            return;
        }
        if let Some(slot) = self.inventory.slots.iter_mut().flatten().find(|i| i.name == name && i.count < i.stack_size) {
            slot.count += count;
            return;
        }
        let Some(def) = self.registry.items_by_name.get(name).cloned() else { return };
        let start = self.inventory.inventory_start;
        let end = self.inventory.inventory_end.min(self.inventory.slots.len());
        for s in start..end {
            if self.inventory.slots[s].is_none() {
                self.inventory.slots[s] = Some(Item {
                    type_id: def.id,
                    count,
                    metadata: 0,
                    nbt: None,
                    name: name.to_string(),
                    display_name: def.display_name,
                    stack_size: def.stack_size,
                    max_durability: def.max_durability,
                    components: vec![],
                    removed_components: vec![],
                });
                return;
            }
        }
    }

    fn sync_current_window(&mut self) {
        if let Some(w) = self.current_window.take() {
            let inv_len = w.inventory_end - w.inventory_start;
            for i in 0..inv_len {
                let cs = w.inventory_start + i;
                let ps = self.inventory.inventory_start + i;
                if cs < w.slots.len() && ps < self.inventory.slots.len() {
                    self.inventory.slots[ps] = w.slots[cs].clone();
                }
            }
        }
    }

    /// Use the held item (right-click in the air).
    pub async fn activate_item(&mut self) -> std::io::Result<()> {
        self.sequence += 1;
        let seq = self.sequence;
        self.client
            .write(
                "use_item",
                PValue::compound(vec![
                    ("hand", PValue::num(0.0)),
                    ("sequence", PValue::num(seq as f64)),
                    (
                        "rotation",
                        PValue::compound(vec![
                            ("x", PValue::num(super::to_notchian_yaw(self.entity.yaw))),
                            ("y", PValue::num(super::to_notchian_pitch(self.entity.pitch))),
                        ]),
                    ),
                ]),
            )
            .await
    }
}

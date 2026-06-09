//! Crafting — recipe lookup and execution via window clicks. Port of
//! typecraft's `bot/crafting.ts`. Supports the 2x2 inventory grid and (with an
//! open crafting table) the 3x3 grid.

use crate::recipe::{find_recipes, Recipe, RecipeItem};
use crate::window::Window;

use super::Bot;

/// Whether an item type satisfies a recipe ingredient (tag-aware).
fn ingredient_matches(type_id: i32, ingredient: &RecipeItem) -> bool {
    type_id == ingredient.id
        || ingredient.choices.as_ref().map(|c| c.contains(&type_id)).unwrap_or(false)
}

/// Find a slot in `window` holding any item that satisfies `ingredient`.
fn find_ingredient_slot(window: &Window, ingredient: &RecipeItem) -> Option<usize> {
    let mut ids = vec![ingredient.id];
    if let Some(choices) = &ingredient.choices {
        ids.extend(choices.iter().copied());
    }
    for id in ids {
        for (i, slot) in window.slots.iter().enumerate() {
            if let Some(item) = slot {
                if item.type_id == id
                    && (ingredient.metadata.is_none() || Some(item.metadata) == ingredient.metadata)
                {
                    return Some(i);
                }
            }
        }
    }
    None
}

impl<'a> Bot<'a> {
    /// Recipes producing `item_type`, optionally filtered by whether a crafting
    /// table is available and a minimum result count.
    pub fn recipes_for(&self, item_type: i32, min_result_count: Option<i32>, crafting_table: bool) -> Vec<Recipe> {
        find_recipes(self.registry, item_type, None)
            .into_iter()
            .filter(|r| {
                if let Some(min) = min_result_count {
                    if r.result.count < min {
                        return false;
                    }
                }
                if !crafting_table && r.requires_table {
                    return false;
                }
                true
            })
            .collect()
    }

    /// Craft `times` of a recipe. `crafting_table` must be `true` (and a table
    /// window open) for 3x3 recipes; 2x2 recipes use the player inventory grid.
    pub async fn craft(&mut self, recipe: &Recipe, times: i32, crafting_table: bool) -> std::io::Result<()> {
        if recipe.requires_table && !crafting_table {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "recipe requires a crafting table"));
        }
        let (w, h) = if crafting_table { (3usize, 3usize) } else { (2usize, 2usize) };
        let slot = |x: usize, y: usize| -> i32 { (1 + x + w * y) as i32 };

        for _ in 0..times.max(1) {
            // Determine which slots the recipe leaves unused (for shapeless placement).
            let mut unused: Vec<i32> = Vec::new();
            if let Some(shape) = &recipe.in_shape {
                for y in 0..h {
                    if let Some(row) = shape.get(y) {
                        for x in 0..row.len() {
                            if row[x].id == -1 {
                                unused.push(slot(x, y));
                            }
                        }
                        for x in row.len()..w {
                            unused.push(slot(x, y));
                        }
                    } else {
                        for x in 0..w {
                            unused.push(slot(x, y));
                        }
                    }
                }
            } else {
                for y in 0..h {
                    for x in 0..w {
                        unused.push(slot(x, y));
                    }
                }
            }

            let mut original_source: Option<i32> = None;

            // Place shaped ingredients.
            if let Some(shape) = &recipe.in_shape {
                for y in 0..shape.len() {
                    let row = &shape[y];
                    for x in 0..row.len() {
                        let ingredient = &row[x];
                        if ingredient.id == -1 {
                            continue;
                        }
                        let held_matches = self
                            .window_selected()
                            .map(|t| ingredient_matches(t, ingredient))
                            .unwrap_or(false);
                        if !held_matches {
                            let src = self
                                .active_window_ref()
                                .and_then(|win| find_ingredient_slot(win, ingredient))
                                .ok_or_else(|| missing(ingredient))?;
                            let src = src as i32;
                            original_source.get_or_insert(src);
                            self.click_window(src, 0, 0).await?;
                        }
                        self.click_window(slot(x, y), 1, 0).await?; // right-click: drop one
                    }
                }
            }

            // Place shapeless ingredients into unused slots.
            if let Some(ingredients) = &recipe.ingredients {
                for ingredient in ingredients {
                    let dest = unused.pop().ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::Other, "no free crafting slots")
                    })?;
                    let held_matches = self
                        .window_selected()
                        .map(|t| ingredient_matches(t, ingredient))
                        .unwrap_or(false);
                    if !held_matches {
                        let src = self
                            .active_window_ref()
                            .and_then(|win| find_ingredient_slot(win, ingredient))
                            .ok_or_else(|| missing(ingredient))?;
                        let src = src as i32;
                        original_source.get_or_insert(src);
                        self.click_window(src, 0, 0).await?;
                    }
                    self.click_window(dest, 1, 0).await?;
                }
            }

            // Return any leftover held item, then take the result + clear the grid.
            let (inv_start, inv_end) = self.active_inventory_range();
            self.put_selected_item_range(inv_start, inv_end, original_source.unwrap_or(0)).await?;

            // Wait for the server to compute and send the crafting result into
            // slot 0 before taking it (else we'd grab an empty slot).
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
            while self.active_slot(0).is_none() && std::time::Instant::now() < deadline {
                if matches!(self.drive_tick().await?, super::DriveStep::Disconnected) {
                    return Ok(());
                }
            }

            self.put_away(0).await?; // take the crafted result

            for s in 0..=(w * h) as i32 {
                if self.active_slot(s as usize).is_some() {
                    self.put_away(s).await?;
                }
            }
        }
        Ok(())
    }

    // ── small accessors used by craft (avoid borrow tangles) ──

    fn window_selected(&self) -> Option<i32> {
        let w = self.current_window.as_ref().unwrap_or(&self.inventory);
        w.selected_item.as_ref().map(|i| i.type_id)
    }

    fn active_window_ref(&self) -> Option<&Window> {
        Some(self.current_window.as_ref().unwrap_or(&self.inventory))
    }

    fn active_inventory_range(&self) -> (usize, usize) {
        let w = self.current_window.as_ref().unwrap_or(&self.inventory);
        (w.inventory_start, w.inventory_end)
    }

    fn active_slot(&self, i: usize) -> Option<&crate::item::Item> {
        let w = self.current_window.as_ref().unwrap_or(&self.inventory);
        w.slots.get(i).and_then(|s| s.as_ref())
    }
}

fn missing(ingredient: &RecipeItem) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("missing crafting ingredient id={}", ingredient.id),
    )
}

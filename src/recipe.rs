//! Crafting recipe parsing and lookup. Port of typecraft's `recipe` module.

use crate::registry::{RawRecipe, RawRecipeItem, Registry};

/// An item in a recipe (ingredient or result).
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeItem {
    pub id: i32,
    pub metadata: Option<i32>,
    pub count: i32,
    /// Tag ingredients accept any of these item ids; `id` is the representative.
    pub choices: Option<Vec<i32>>,
}

/// A parsed crafting recipe.
#[derive(Debug, Clone)]
pub struct Recipe {
    pub result: RecipeItem,
    pub in_shape: Option<Vec<Vec<RecipeItem>>>,
    pub out_shape: Option<Vec<Vec<RecipeItem>>>,
    pub ingredients: Option<Vec<RecipeItem>>,
    pub delta: Vec<RecipeItem>,
    pub requires_table: bool,
}

fn parse_item(raw: &RawRecipeItem) -> RecipeItem {
    match raw {
        RawRecipeItem::None => RecipeItem {
            id: -1,
            metadata: None,
            count: 1,
            choices: None,
        },
        RawRecipeItem::Id(id) => RecipeItem {
            id: *id,
            metadata: None,
            count: 1,
            choices: None,
        },
        RawRecipeItem::Detailed {
            id,
            metadata,
            choices,
        } => RecipeItem {
            id: *id,
            metadata: *metadata,
            count: 1,
            choices: choices.clone(),
        },
    }
}

fn parse_shape(shape: &[Vec<RawRecipeItem>]) -> Vec<Vec<RecipeItem>> {
    shape
        .iter()
        .map(|row| row.iter().map(parse_item).collect())
        .collect()
}

fn parse_ingredients(ingredients: &[RawRecipeItem]) -> Vec<RecipeItem> {
    ingredients
        .iter()
        .map(|raw| RecipeItem {
            count: -1,
            ..parse_item(raw)
        })
        .collect()
}

/// Whether a recipe needs a 3×3 table (vs the 2×2 inventory grid).
fn compute_requires_table(recipe: &Recipe) -> bool {
    let mut space_left: i32 = 4;
    if let Some(in_shape) = &recipe.in_shape {
        if in_shape.len() > 2 {
            return true;
        }
        for row in in_shape {
            if row.len() > 2 {
                return true;
            }
            for item in row {
                if item.id != -1 {
                    space_left -= 1;
                }
            }
        }
    }
    if let Some(ingredients) = &recipe.ingredients {
        space_left -= ingredients.len() as i32;
    }
    space_left < 0
}

/// Net inventory delta from crafting this recipe.
fn compute_delta(recipe: &Recipe) -> Vec<RecipeItem> {
    let mut delta: Vec<RecipeItem> = Vec::new();

    let add = |delta: &mut Vec<RecipeItem>, item: RecipeItem| {
        if let Some(existing) = delta
            .iter_mut()
            .find(|d| d.id == item.id && d.metadata == item.metadata)
        {
            existing.count += item.count;
        } else {
            delta.push(item);
        }
    };

    let apply_shape = |delta: &mut Vec<RecipeItem>, shape: &[Vec<RecipeItem>], direction: i32| {
        for row in shape {
            for item in row {
                if item.id != -1 {
                    add(
                        delta,
                        RecipeItem {
                            count: direction,
                            ..item.clone()
                        },
                    );
                }
            }
        }
    };

    if let Some(in_shape) = &recipe.in_shape {
        apply_shape(&mut delta, in_shape, -1);
    }
    if let Some(out_shape) = &recipe.out_shape {
        apply_shape(&mut delta, out_shape, 1);
    }
    if let Some(ingredients) = &recipe.ingredients {
        for item in ingredients {
            add(&mut delta, item.clone());
        }
    }
    add(&mut delta, recipe.result.clone());

    delta
}

/// Parse a raw recipe into a `Recipe`, computing its delta and table requirement.
pub fn parse_recipe(raw: &RawRecipe) -> Recipe {
    let result = RecipeItem {
        id: raw.result.id,
        metadata: raw.result.metadata,
        count: raw.result.count,
        choices: None,
    };
    let in_shape = raw.in_shape.as_deref().map(parse_shape);
    let out_shape = raw.out_shape.as_deref().map(parse_shape);
    let ingredients = raw.ingredients.as_deref().map(parse_ingredients);

    let mut recipe = Recipe {
        result,
        in_shape,
        out_shape,
        ingredients,
        delta: Vec::new(),
        requires_table: false,
    };
    recipe.delta = compute_delta(&recipe);
    recipe.requires_table = compute_requires_table(&recipe);
    recipe
}

/// Find all recipes that produce the given item id.
pub fn find_recipes(registry: &Registry, item_id: i32, metadata: Option<i32>) -> Vec<Recipe> {
    let Some(raw_recipes) = registry.recipes.get(&item_id) else {
        return Vec::new();
    };
    raw_recipes
        .iter()
        .filter(|raw| match metadata {
            None => true,
            Some(_) => raw.result.metadata.is_none() || raw.result.metadata == metadata,
        })
        .map(parse_recipe)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i32) -> RawRecipeItem {
        RawRecipeItem::Id(id)
    }

    fn raw_shaped(in_shape: Vec<Vec<RawRecipeItem>>, id: i32, count: i32) -> RawRecipe {
        RawRecipe {
            in_shape: Some(in_shape),
            out_shape: None,
            ingredients: None,
            result: crate::registry::RecipeResult {
                id,
                count,
                metadata: None,
            },
        }
    }

    fn raw_shapeless(ingredients: Vec<RawRecipeItem>, id: i32, count: i32) -> RawRecipe {
        RawRecipe {
            in_shape: None,
            out_shape: None,
            ingredients: Some(ingredients),
            result: crate::registry::RecipeResult {
                id,
                count,
                metadata: None,
            },
        }
    }

    #[test]
    fn parses_shapeless() {
        let recipe = parse_recipe(&raw_shapeless(vec![item(4), item(804)], 2, 1));
        assert_eq!(recipe.result.id, 2);
        assert_eq!(recipe.result.count, 1);
        assert!(recipe.in_shape.is_none());
        let ingredients = recipe.ingredients.unwrap();
        assert_eq!(ingredients.len(), 2);
        assert_eq!(ingredients[0].id, 4);
        assert_eq!(ingredients[0].count, -1);
        assert_eq!(ingredients[1].id, 804);
    }

    #[test]
    fn parses_shaped() {
        let recipe = parse_recipe(&raw_shaped(
            vec![vec![item(2), item(2)], vec![item(2), item(2)]],
            3,
            4,
        ));
        assert_eq!(recipe.result.count, 4);
        let in_shape = recipe.in_shape.unwrap();
        assert_eq!(in_shape.len(), 2);
        assert_eq!(in_shape[0][0].id, 2);
        assert_eq!(in_shape[0][0].count, 1);
        assert!(recipe.ingredients.is_none());
    }

    #[test]
    fn parses_null_as_minus_one() {
        let recipe = parse_recipe(&raw_shaped(
            vec![
                vec![item(1), RawRecipeItem::None],
                vec![RawRecipeItem::None, item(1)],
            ],
            5,
            1,
        ));
        assert_eq!(recipe.in_shape.unwrap()[0][1].id, -1);
    }

    #[test]
    fn parses_metadata() {
        let recipe = parse_recipe(&RawRecipe {
            in_shape: Some(vec![vec![
                RawRecipeItem::Detailed {
                    id: 3,
                    metadata: Some(0),
                    choices: None,
                },
                item(13),
            ]]),
            out_shape: None,
            ingredients: None,
            result: crate::registry::RecipeResult {
                id: 3,
                count: 4,
                metadata: Some(1),
            },
        });
        assert_eq!(recipe.in_shape.unwrap()[0][0].metadata, Some(0));
        assert_eq!(recipe.result.metadata, Some(1));
    }

    #[test]
    fn requires_table_logic() {
        assert!(
            !parse_recipe(&raw_shaped(
                vec![vec![item(2), item(2)], vec![item(2), item(2)]],
                3,
                4
            ))
            .requires_table
        );
        assert!(
            parse_recipe(&raw_shaped(vec![vec![item(1), item(2), item(3)]], 5, 1)).requires_table
        );
        assert!(
            parse_recipe(&raw_shaped(
                vec![vec![item(1)], vec![item(2)], vec![item(3)]],
                5,
                1
            ))
            .requires_table
        );
        assert!(
            parse_recipe(&raw_shapeless(
                vec![item(1), item(2), item(3), item(4), item(5)],
                10,
                1
            ))
            .requires_table
        );
        assert!(!parse_recipe(&raw_shapeless(vec![item(1), item(2)], 10, 1)).requires_table);
    }

    #[test]
    fn delta_shapeless() {
        let recipe = parse_recipe(&raw_shapeless(vec![item(4), item(804)], 2, 1));
        assert!(recipe.delta.iter().any(|d| d.id == 4 && d.count == -1));
        assert!(recipe.delta.iter().any(|d| d.id == 804 && d.count == -1));
        assert!(recipe.delta.iter().any(|d| d.id == 2 && d.count == 1));
    }

    #[test]
    fn delta_shaped() {
        let recipe = parse_recipe(&raw_shaped(
            vec![vec![item(2), item(2)], vec![item(2), item(2)]],
            3,
            4,
        ));
        assert_eq!(recipe.delta.iter().find(|d| d.id == 2).unwrap().count, -4);
        assert_eq!(recipe.delta.iter().find(|d| d.id == 3).unwrap().count, 4);
    }

    #[test]
    fn find_recipes_unknown_returns_empty() {
        let reg = Registry::build(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            crate::registry::BlockCollisionShapes::default(),
            std::collections::HashMap::new(),
            "26.1.2",
        );
        assert!(find_recipes(&reg, 999999, None).is_empty());
    }
}

//! Journal behaviour library (ids are stored in the DB; labels live here).

pub struct Item {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
}

pub const LIBRARY: &[Item] = &[
    Item { id: "alcohol", label: "Had alcohol", category: "Nutrition" },
    Item { id: "caffeine_late", label: "Caffeine after 2 pm", category: "Nutrition" },
    Item { id: "late_meal", label: "Ate a late meal", category: "Nutrition" },
    Item { id: "screens_bed", label: "Screens in bed", category: "Sleep habits" },
    Item { id: "stretching", label: "Stretched or did mobility", category: "Wellness" },
    Item { id: "sauna", label: "Used a sauna", category: "Wellness" },
    Item { id: "magnesium", label: "Took magnesium", category: "Supplements" },
    Item { id: "travel", label: "Travelled", category: "Lifestyle" },
    Item { id: "illness", label: "Felt unwell", category: "Health" },
    Item { id: "meditation", label: "Meditated", category: "Wellness" },
];

pub const DEFAULT_SELECTED: &[&str] = &["alcohol", "caffeine_late", "late_meal", "screens_bed", "stretching", "sauna"];

pub fn find(id: &str) -> Option<&'static Item> {
    LIBRARY.iter().find(|i| i.id == id)
}

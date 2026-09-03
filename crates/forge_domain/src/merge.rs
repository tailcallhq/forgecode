use ::std::collections::HashMap;
use ::std::hash::Hash;

pub mod vec {

    use std::collections::HashMap;

    use super::Key;

    #[allow(unused)]
    pub fn unify_by_key<T: Key>(
        base: &mut Vec<T>,
        other: Vec<T>,
        mut merge_from: impl FnMut(&mut T, T),
    ) where
        T::Id: Clone + std::hash::Hash + Eq,
    {
        // Create a HashMap for O(1) lookup of base agents by their key
        let mut base_map: HashMap<T::Id, usize> = HashMap::new();
        for (index, agent) in base.iter().enumerate() {
            base_map.insert(agent.key().clone(), index);
        }

        for other_agent in other {
            if let Some(&index) = base_map.get(other_agent.key()) {
                // If the base contains an agent with the same Key, merge them
                if let Some(base_agent) = base.get_mut(index) {
                    merge_from(base_agent, other_agent);
                }
            } else {
                let key = other_agent.key().clone();
                // Otherwise, append the other agent to the base list
                base.push(other_agent);
                base_map.insert(key, base.len() - 1);
            }
        }
    }
}

#[allow(unused)]
pub trait Key {
    type Id: Eq;
    fn key(&self) -> &Self::Id;
}

#[allow(unused)]
pub fn hashmap<K: Eq + Hash, V>(base: &mut HashMap<K, V>, other: HashMap<K, V>) {
    for (key, value) in other {
        base.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::Key;
    use super::vec::unify_by_key;

    #[derive(Debug, PartialEq)]
    struct Item(&'static str, usize);

    impl Key for Item {
        type Id = &'static str;

        fn key(&self) -> &Self::Id {
            &self.0
        }
    }

    #[test]
    fn duplicate_incoming_keys_merge_into_the_first_appended_item() {
        let mut fixture = vec![Item("base", 1)];
        let other = vec![Item("new", 2), Item("new", 3)];

        unify_by_key(&mut fixture, other, |base, incoming| {
            base.1 += incoming.1;
        });

        let expected = vec![Item("base", 1), Item("new", 5)];
        assert_eq!(fixture, expected);
    }
}

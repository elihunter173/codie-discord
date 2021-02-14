use std::convert::TryInto;

use serenity::model::id::MessageId;
use sled::Tree;

pub struct MessageIds(Tree);

// TODO: There's some duplicated code, maybe use traits to make it generic?
impl MessageIds {
    pub fn new(tree: Tree) -> Self {
        Self(tree)
    }

    pub fn insert(&self, k: MessageId, v: MessageId) -> sled::Result<Option<MessageId>> {
        self.0
            .insert(&k.as_u64().to_le_bytes(), &v.as_u64().to_le_bytes())
            .map(|opt| {
                opt.map(|ivec| MessageId(u64::from_le_bytes(ivec.as_ref().try_into().unwrap())))
            })
    }

    pub fn get(&self, k: MessageId) -> sled::Result<Option<MessageId>> {
        self.0.get(&k.as_u64().to_le_bytes()).map(|opt| {
            opt.map(|ivec| MessageId(u64::from_le_bytes(ivec.as_ref().try_into().unwrap())))
        })
    }
}

use crossbeam_queue::SegQueue;

use crate::{prelude::*, world::EntitiesRes};

struct Queue<T>(SegQueue<T>);

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self(SegQueue::new())
    }
}

#[cfg(feature = "parallel")]
mod traits_internal {
    use crate::{Component, Entity, World};

    pub trait LazyUpdateInternal: Send + Sync {
        fn update(self: Box<Self>, world: &mut World);
    }

    pub trait FnLazyUpdate: FnOnce(&World) + Send + Sync + 'static {}
    impl<F> FnLazyUpdate for F where F: FnOnce(&World) + Send + Sync + 'static {}

    pub trait FnLazyUpdateMut: FnOnce(&mut World) + Send + Sync + 'static {}
    impl<F> FnLazyUpdateMut for F where F: FnOnce(&mut World) + Send + Sync + 'static {}

    pub trait ComponentBound: Component + Send + Sync {}
    impl<C> ComponentBound for C where C: Component + Send + Sync {}

    pub trait IntoIteratorBound<C>:
        IntoIterator<Item = (Entity, C)> + Send + Sync + 'static
    {
    }
    impl<I, C> IntoIteratorBound<C> for I
    where
        I: IntoIterator<Item = (Entity, C)> + Send + Sync + 'static,
        C: ComponentBound,
    {
    }
}

#[cfg(not(feature = "parallel"))]
mod traits_internal {
    use crate::{Component, Entity, World};

    pub trait LazyUpdateInternal {
        fn update(self: Box<Self>, world: &mut World);
    }

    pub trait FnLazyUpdate: FnOnce(&World) + 'static {}
    impl<F> FnLazyUpdate for F where F: FnOnce(&World) + 'static {}

    pub trait FnLazyUpdateMut: FnOnce(&mut World) + 'static {}
    impl<F> FnLazyUpdateMut for F where F: FnOnce(&mut World) + 'static {}

    pub trait ComponentBound: Component {}
    impl<C> ComponentBound for C where C: Component {}

    pub trait IntoIteratorBound<C>: IntoIterator<Item = (Entity, C)> + 'static {}
    impl<C, I> IntoIteratorBound<C> for I
    where
        I: IntoIterator<Item = (Entity, C)> + 'static,
        C: ComponentBound,
    {
    }
}

use traits_internal::*;

/// Like `EntityBuilder`, but inserts the component
/// lazily, meaning on `maintain`.
/// If you need those components to exist immediately,
/// you have to insert them into the storages yourself.
#[must_use = "Please call .build() on this to finish building it."]
pub struct LazyBuilder<'a> {
    /// The entity that we're inserting components for.
    pub entity: Entity,
    /// The lazy update reference.
    pub lazy: &'a LazyUpdate,
}

impl<'a> Builder for LazyBuilder<'a> {
    /// Inserts a component using [LazyUpdate].
    ///
    /// If a component was already associated with the entity, it will
    /// overwrite the previous component.
    fn with<C>(self, component: C) -> Self
    where
        C: ComponentBound,
    {
        let entity = self.entity;
        self.lazy.exec(move |world: &World| {
            let mut storage: WriteStorage<C> = SystemData::fetch(world);
            if storage.insert(entity, component).is_err() {
                log::warn!(
                    "Lazy insert of component failed because {:?} was dead.",
                    entity
                );
            }
        });

        self
    }

    /// Finishes the building and returns the built entity.
    /// Please note that no component is associated to this
    /// entity until you call [`World::maintain`].
    fn build(self) -> Entity {
        self.entity
    }
}

impl<F> LazyUpdateInternal for F
where
    F: FnLazyUpdateMut,
{
    fn update(self: Box<Self>, world: &mut World) {
        self(world);
    }
}

/// Lazy updates can be used for world updates
/// that need to borrow a lot of resources
/// and as such should better be done at the end.
/// They work lazily in the sense that they are
/// dispatched when calling `world.maintain()`.
///
/// Lazy updates are dispatched in the order that they
/// are requested. Multiple updates sent from one system
/// may be overridden by updates sent from other systems.
///
/// Please note that the provided methods take `&self`
/// so there's no need to get `LazyUpdate` mutably.
/// This resource is added to the world by default.
pub struct LazyUpdate {
    queue: Option<Queue<Box<dyn LazyUpdateInternal>>>,
}

impl Default for LazyUpdate {
    fn default() -> Self {
        Self {
            queue: Some(Default::default()),
        }
    }
}

impl LazyUpdate {
    /// Creates a new `LazyBuilder` which inserts components
    /// using `LazyUpdate`. This means that the components won't
    /// be available immediately, but only after a `maintain`
    /// on `World` is performed.
    ///
    /// ## Examples
    ///
    /// ```
    /// # use specs::prelude::*;
    /// # let mut world = World::new();
    /// struct Pos(f32, f32);
    ///
    /// impl Component for Pos {
    ///     type Storage = VecStorage<Self>;
    /// }
    ///
    /// # let lazy = world.read_resource::<LazyUpdate>();
    /// # let entities = world.entities();
    /// let my_entity = lazy.create_entity(&entities).with(Pos(1.0, 3.0)).build();
    /// ```
    pub fn create_entity(&self, ent: &EntitiesRes) -> LazyBuilder {
        let entity = ent.create();

        LazyBuilder { entity, lazy: self }
    }

    /// Lazily inserts a component for an entity.
    ///
    /// ## Examples
    ///
    /// ```
    /// # use specs::prelude::*;
    /// #
    /// struct Pos(f32, f32);
    ///
    /// impl Component for Pos {
    ///     type Storage = VecStorage<Self>;
    /// }
    ///
    /// struct InsertPos;
    ///
    /// impl<'a> System<'a> for InsertPos {
    ///     type SystemData = (Entities<'a>, Read<'a, LazyUpdate>);
    ///
    ///     fn run(&mut self, (ent, lazy): Self::SystemData) {
    ///         let a = ent.create();
    ///         lazy.insert(a, Pos(1.0, 1.0));
    ///     }
    /// }
    /// ```
    pub fn insert<C>(&self, e: Entity, c: C)
    where
        C: ComponentBound,
    {
        self.exec(move |world: &World| {
            let mut storage: WriteStorage<C> = SystemData::fetch(world);
            if storage.insert(e, c).is_err() {
                log::warn!("Lazy insert of component failed because {:?} was dead.", e);
            }
        });
    }

    /// Lazily inserts components for entities.
    ///
    /// ## Examples
    ///
    /// ```
    /// # use specs::prelude::*;
    /// #
    /// struct Pos(f32, f32);
    ///
    /// impl Component for Pos {
    ///     type Storage = VecStorage<Self>;
    /// }
    ///
    /// struct InsertPos;
    ///
    /// impl<'a> System<'a> for InsertPos {
    ///     type SystemData = (Entities<'a>, Read<'a, LazyUpdate>);
    ///
    ///     fn run(&mut self, (ent, lazy): Self::SystemData) {
    ///         let a = ent.create();
    ///         let b = ent.create();
    ///
    ///         lazy.insert_all(vec![(a, Pos(3.0, 1.0)), (b, Pos(0.0, 4.0))]);
    ///     }
    /// }
    /// ```
    pub fn insert_all<C, I>(&self, iter: I)
    where
        C: ComponentBound,
        I: IntoIteratorBound<C>,
    {
        self.exec(move |world: &World| {
            let mut storage: WriteStorage<C> = SystemData::fetch(world);
            for (e, c) in iter {
                if storage.insert(e, c).is_err() {
                    log::warn!("Lazy insert of component failed because {:?} was dead.", e);
                }
            }
        });
    }

    /// Lazily removes a component.
    ///
    /// ## Examples
    ///
    /// ```
    /// # use specs::prelude::*;
    /// #
    /// struct Pos;
    ///
    /// impl Component for Pos {
    ///     type Storage = VecStorage<Self>;
    /// }
    ///
    /// struct RemovePos;
    ///
    /// impl<'a> System<'a> for RemovePos {
    ///     type SystemData = (Entities<'a>, Read<'a, LazyUpdate>);
    ///
    ///     fn run(&mut self, (ent, lazy): Self::SystemData) {
    ///         for entity in ent.join() {
    ///             lazy.remove::<Pos>(entity);
    ///         }
    ///     }
    /// }
    /// ```
    pub fn remove<C>(&self, e: Entity)
    where
        C: ComponentBound,
    {
        self.exec(move |world: &World| {
            let mut storage: WriteStorage<C> = SystemData::fetch(world);
            storage.remove(e);
        });
    }

    /// Lazily executes a closure with world access.
    ///
    /// ## Examples
    ///
    /// ```
    /// # use specs::prelude::*;
    /// #
    /// struct Pos;
    ///
    /// impl Component for Pos {
    ///     type Storage = VecStorage<Self>;
    /// }
    ///
    /// struct Execution;
    ///
    /// impl<'a> System<'a> for Execution {
    ///     type SystemData = (Entities<'a>, Read<'a, LazyUpdate>);
    ///
    ///     fn run(&mut self, (ent, lazy): Self::SystemData) {
    ///         for entity in ent.join() {
    ///             lazy.exec(move |world| {
    ///                 if world.is_alive(entity) {
    ///                     println!("Entity {:?} is alive.", entity);
    ///                 }
    ///             });
    ///         }
    ///     }
    /// }
    /// ```
    pub fn exec<F>(&self, f: F)
    where
        F: FnLazyUpdate,
    {
        self.queue
            .as_ref()
            .unwrap()
            .0
            .push(Box::new(|w: &mut World| f(w)));
    }

    /// Lazily executes a closure with mutable world access.
    ///
    /// This can be used to add a resource to the `World` from a system.
    ///
    /// ## Examples
    ///
    /// ```
    /// # use specs::prelude::*;
    /// #
    ///
    /// struct Sys;
    ///
    /// impl<'a> System<'a> for Sys {
    ///     type SystemData = (Entities<'a>, Read<'a, LazyUpdate>);
    ///
    ///     fn run(&mut self, (ent, lazy): Self::SystemData) {
    ///         for entity in ent.join() {
    ///             lazy.exec_mut(move |world| {
    ///                 // complete extermination!
    ///                 world.delete_all();
    ///             });
    ///         }
    ///     }
    /// }
    /// ```
    pub fn exec_mut<F>(&self, f: F)
    where
        F: FnLazyUpdateMut,
    {
        self.queue.as_ref().unwrap().0.push(Box::new(f));
    }

    /// Allows to temporarily take the inner queue.
    pub(super) fn take(&mut self) -> Self {
        Self {
            queue: self.queue.take(),
        }
    }

    /// Needs to be called to restore the inner queue.
    pub(super) fn restore(&mut self, mut maintained: Self) {
        use std::mem::swap;

        swap(&mut self.queue, &mut maintained.queue);
    }

    pub(super) fn maintain(&mut self, world: &mut World) {
        let lazy = &mut self.queue.as_mut().unwrap().0;

        while let Ok(l) = lazy.pop() {
            l.update(world);
        }
    }
}

impl Drop for LazyUpdate {
    fn drop(&mut self) {
        // TODO: remove as soon as leak is fixed in crossbeam
        if let Some(queue) = self.queue.as_mut() {
            while queue.0.pop().is_ok() {}
        }
    }
}

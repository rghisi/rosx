use alloc::vec::Vec;
use alloc::collections::VecDeque;
use task::{SharedTask, Task};
use task_arena::Error::TaskNotFound;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TaskHandle {
    pub index: u8,
    pub generation: u8,
}

#[derive(Debug)]
pub enum Error {
    TaskNotFound,
}

pub struct GenArena {
    tasks: Vec<Option<SharedTask>>,
    generations: Vec<u8>,
    free_slots: VecDeque<u8>,
}

impl GenArena {

    pub fn new(initial_capacity: usize) -> Self {
        assert!(
            initial_capacity > 0 && initial_capacity <= 256,
            "TaskArena capacity must be between 1 and 256"
        );

        let mut tasks = Vec::with_capacity(initial_capacity);
        let mut generations = Vec::with_capacity(initial_capacity);
        let mut free_slots = VecDeque::with_capacity(initial_capacity);
        for slot in 0..initial_capacity {
            tasks.push(None);
            generations.push(0);
            free_slots.push_back(slot as u8);
        }

        GenArena {
            tasks,
            generations,
            free_slots
        }
    }

    pub fn add(&mut self, task: SharedTask) -> Result<TaskHandle, Error> {
        if self.free_slots.is_empty() {
            let increment = self.generations.capacity();
            let new_size = increment + increment;
            for i in increment..new_size {
                self.tasks.push(None);
                self.generations.push(0);
                self.free_slots.push_back(i as u8);
            }
        }

        let index = self.free_slots.pop_front().unwrap() as usize;
        let generation = self.generations[index];
        self.tasks[index] = Some(task);

        Ok(TaskHandle {
            index: index as u8,
            generation,
        })
    }

    pub fn borrow(&self, handle: TaskHandle) -> Result<&Task, Error> {
        if self.generations[handle.index as usize] == handle.generation {
            return Ok(self.tasks[handle.index as usize].as_deref().unwrap());
        }

        Err(TaskNotFound)
    }

    pub fn borrow_mut(&mut self, handle: TaskHandle) -> Result<&mut Task, Error> {
        if self.generations[handle.index as usize] == handle.generation {
            return Ok(self.tasks[handle.index as usize].as_deref_mut().unwrap());
        }

        Err(TaskNotFound)
    }

    pub fn remove(&mut self, handle: TaskHandle) -> Result<SharedTask, Error> {
        let index = handle.index as usize;
        let generation = handle.generation;
        if generation == self.generations[index] {
            let task = self.tasks[handle.index as usize].take().unwrap();
            self.tasks[handle.index as usize] = None;
            let next_generation = generation + 1;
            self.generations[index] = next_generation;
            self.free_slots.push_back(index as u8);

            Ok(task)
        } else {
            Err(TaskNotFound)
        }
    }
}
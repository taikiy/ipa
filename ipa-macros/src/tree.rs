use std::{
    cell::RefCell,
    fmt::Debug,
    ops::Deref,
    rc::{Rc, Weak},
};

#[derive(Clone, Debug)]
pub struct NodeData<T> {
    value: T,
    parent: Parent<T>,
    children: Children<T>,
}

impl<T> Deref for NodeData<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

type NodeDataRef<T> = Rc<NodeData<T>>;
type NodeDataWeakRef<T> = Weak<NodeData<T>>;

type Parent<T> = RefCell<NodeDataWeakRef<T>>;
type Children<T> = RefCell<Vec<NodeDataRef<T>>>;

#[derive(Clone, Debug)]
pub struct Node<T> {
    data: NodeDataRef<T>,
}

impl<T> Node<T> {
    pub fn new(value: T) -> Node<T> {
        let new_node = NodeData {
            value,
            parent: RefCell::new(Weak::new()),
            children: RefCell::new(Vec::new()),
        };
        Node {
            data: Rc::new(new_node),
        }
    }

    fn get_copy(&self) -> NodeDataRef<T> {
        Rc::clone(&self.data)
    }

    pub fn add_child(&self, value: T) -> Node<T> {
        let new_child = Node::new(value);
        {
            let mut my_children = self.data.children.borrow_mut();
            my_children.push(new_child.get_copy());
        } // drop the borrow
        {
            let mut childs_parent = new_child.data.parent.borrow_mut();
            *childs_parent = Rc::downgrade(&self.get_copy());
        } // drop the borrow
        new_child
    }

    pub fn get_children(&self) -> Vec<Node<T>> {
        self.children
            .borrow()
            .iter()
            .map(|x| Node { data: Rc::clone(x) })
            .collect::<Vec<_>>()
    }

    pub fn get_parent(&self) -> Option<Node<T>> {
        let my_parent_weak = self.parent.borrow();
        if let Some(my_parent_ref) = my_parent_weak.upgrade() {
            Some(Node {
                data: my_parent_ref,
            })
        } else {
            None
        }
    }
}

impl<T> Deref for Node<T> {
    type Target = NodeData<T>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

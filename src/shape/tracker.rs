use itertools::Itertools;

use super::symbolic::*;

fn expr_node(idx: Node, shape: &[usize], strides: &[usize]) -> Node {
    let mut acc = 1;
    let mut ret = vec![];
    for (d, s) in shape.iter().zip(strides.iter()).rev() {
        ret.push(((idx.clone() / (acc as i32)) % (*d as i32)) * (*s as i32));
        acc *= d;
    }

    Node::sum(ret)
}

#[derive(Debug, Clone)]
pub struct View {
    shape: Vec<usize>,
    strides: Vec<usize>,
}

fn merge_views(v2: &View, v1: &View) -> Option<View> {
    let idxs = v1
        .shape
        .iter()
        .enumerate()
        .map(|(i, s)| Node::variable(format!("idx{i}"), 0, (s - 1) as i32))
        .collect::<Vec<_>>();
    let idx = Node::sum(
        idxs.clone()
            .into_iter()
            .zip(v1.shape.iter())
            .zip(v1.strides.iter())
            .filter(|((_, sh), st)| **sh != 1 && **st != 0)
            .map(|((i, _), st)| i * *st as i32)
            .collect_vec(),
    );

    let idx = expr_node(idx, &v2.shape, &v2.strides);
    let mut ret = vec![0; idxs.len()];
    for node in if let NodeType::RedNode(RedOp::Sum, n) = idx.node_type {
        n
    } else {
        vec![idx]
    } {
        if let NodeType::OpNode(Op::Mul, a) = &node.node_type {
            if matches!(a.node_type, NodeType::Variable(_)) {
                ret[idxs.iter().position(|i| *i == **a).unwrap()] = node.b as usize;
            } else if matches!(node.node_type, NodeType::Variable(_)) {
                ret[idxs.iter().position(|i| *i == node).unwrap()] = 1;
            }
        } else if matches!(node.node_type, NodeType::Variable(_)) {
            ret[idxs.iter().position(|i| *i == node).unwrap()] = 1;
        }
    }
    if ret.iter().any(|i| *i == 0) {
        None
    } else {
        Some(View {
            shape: v1.shape.clone(),
            strides: ret,
        })
    }
}

pub fn default_strides(shape: &[usize]) -> Vec<usize> {
    let mut acc = 1;
    let mut strides = shape.to_vec();
    for i in strides.iter_mut().rev() {
        let tmp = *i;
        *i = acc;
        acc *= tmp;
    }

    strides
}

#[derive(Debug, Clone)]
pub struct ShapeTracker {
    views: Vec<View>,
}

impl ShapeTracker {
    pub fn new(shape: Vec<usize>) -> Self {
        Self {
            views: vec![View {
                strides: default_strides(&shape),
                shape,
            }],
        }
    }

    pub fn shape(&self) -> &Vec<usize> {
        &self.views.last().unwrap().shape
    }

    pub fn reshape(&mut self, new_shape: Vec<usize>) {
        let new_view = View {
            strides: default_strides(&new_shape),
            shape: new_shape,
        };

        self.views.push(new_view);
        self.simplify();
    }

    fn simplify(&mut self) {
        while self.views.len() > 1 {
            if let Some(merged) = merge_views(
                &self.views[self.views.len() - 2],
                &self.views[self.views.len() - 1],
            ) {
                self.views.pop();
                *self.views.last_mut().unwrap() = merged;
            } else {
                break;
            }
        }
    }

    pub fn permute(&mut self, new_dims: &[usize]) {
        let view = self.views.last_mut().unwrap();
        let (old_shape, old_strides) = (view.shape.clone(), view.strides.clone());
        for (i, j) in new_dims.iter().enumerate() {
            view.shape[i] = old_shape[*j];
            view.strides[i] = old_strides[*j];
        }
    }

    pub fn index_fn(&self) -> impl Fn(usize) -> usize {
        // Get expression
        let mut idx = Node::variable(
            "idx".to_string(),
            0,
            self.shape().iter().product::<usize>() as i32,
        );
        for view in self.views.iter().rev() {
            idx = expr_node(idx, &view.shape, &view.strides);
        }

        // Turn expression into function by unwrapping it into a series of function chains
        let mut ops_and_nums = vec![];
        let mut node = &idx;
        loop {
            match &node.node_type {
                NodeType::OpNode(op, a) => {
                    ops_and_nums.push((
                        match op {
                            Op::Div => std::ops::Div::<i32>::div,
                            Op::Mul => std::ops::Mul::<i32>::mul,
                            Op::Mod => std::ops::Rem::<i32>::rem,
                        },
                        node.b,
                    ));
                    node = a.as_ref();
                }
                NodeType::Variable(_) => break,
                NodeType::Num => panic!("Num node encountered"),
                NodeType::RedNode(_, _) => panic!("Rednode encountered"),
            }
        }
        ops_and_nums.reverse();

        move |i| {
            let mut i = i as i32;
            for (op, num) in &ops_and_nums {
                i = (op)(i, *num);
            }
            i as usize
        }
    }
}
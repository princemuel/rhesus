package ast

import (
	"bytes"
	"monkey/token"
	"strings"
)

// The base Node interface. It means anything in the syntax tree.
type Node interface {
	TokenLiteral() string
	String() string
}

// Statement is a Node that performs an action without producing a value
// (let, return, ...).
//
// # All statement nodes implement this
//
// StmtNode is a marker method: it has no behavior and exists only so the
// compiler can tell statements from expressions.
type Stmt interface {
	Node
	StmtNode()
}

// Expression is a Node that produces a value (5, x, a + b, ...).
//
// # All expression nodes implement this
//
// ExprNode is a marker method, like StmtNode.
type Expr interface {
	Node
	ExprNode()
}

type Program struct {
	Statements []Stmt
}

func (p *Program) TokenLiteral() string {
	if len(p.Statements) == 0 {
		return ""
	}
	return p.Statements[0].TokenLiteral()
}

func (p *Program) String() string {
	var out bytes.Buffer

	for _, s := range p.Statements {
		out.WriteString(s.String())
	}

	return out.String()
}

// LetStmt is `let <ident> = <value>;`.
type LetStmt struct {
	Token token.Token // the LET token
	Name  *Identifier
	Value Expr
}

func (*LetStmt) StmtNode()               {}
func (ls *LetStmt) TokenLiteral() string { return ls.Token.Literal }
func (ls *LetStmt) String() string {
	var out bytes.Buffer

	out.WriteString(ls.TokenLiteral())
	out.WriteString(" ")
	out.WriteString(ls.Name.String())
	out.WriteString(" = ")

	if ls.Value != nil {
		out.WriteString(ls.Value.String())
	}

	out.WriteString(";")

	return out.String()
}

type ReturnStatement struct {
	Token       token.Token // the RETURN token
	ReturnValue Expr
}

func (*ReturnStatement) StmtNode()               {}
func (rs *ReturnStatement) TokenLiteral() string { return rs.Token.Literal }
func (rs *ReturnStatement) String() string {
	var out bytes.Buffer

	out.WriteString(rs.TokenLiteral())
	out.WriteString(" ")

	if rs.ReturnValue != nil {
		out.WriteString(rs.ReturnValue.String())
	}

	out.WriteString(";")

	return out.String()
}

type ExpressionStatement struct {
	Token      token.Token // the first token of the expression
	Expression Expr
}

func (*ExpressionStatement) StmtNode()               {}
func (es *ExpressionStatement) TokenLiteral() string { return es.Token.Literal }
func (es *ExpressionStatement) String() string {
	if es.Expression == nil {
		return ""
	}
	return es.Expression.String()
}

type BlockStatement struct {
	Token      token.Token // the { token
	Statements []Stmt
}

func (*BlockStatement) StmtNode()               {}
func (bs *BlockStatement) TokenLiteral() string { return bs.Token.Literal }
func (bs *BlockStatement) String() string {
	var out bytes.Buffer

	for _, s := range bs.Statements {
		out.WriteString(s.String())
	}

	return out.String()
}

// Identifier is a ident, such as `x` or `add`.
type Identifier struct {
	Token token.Token // the IDENT token
	Value string
}

func (*Identifier) ExprNode()              {}
func (i *Identifier) TokenLiteral() string { return i.Token.Literal }
func (i *Identifier) String() string       { return i.Value }

type Boolean struct {
	Token token.Token
	Value bool
}

func (*Boolean) ExprNode()              {}
func (b *Boolean) TokenLiteral() string { return b.Token.Literal }
func (b *Boolean) String() string       { return b.Token.Literal }

type IntegerLiteral struct {
	Token token.Token
	Value int64
}

func (*IntegerLiteral) ExprNode()               {}
func (il *IntegerLiteral) TokenLiteral() string { return il.Token.Literal }
func (il *IntegerLiteral) String() string       { return il.Token.Literal }

type PrefixExpression struct {
	Token    token.Token // The prefix token, e.g. !
	Operator string
	Right    Expr
}

func (*PrefixExpression) ExprNode()               {}
func (pe *PrefixExpression) TokenLiteral() string { return pe.Token.Literal }
func (pe *PrefixExpression) String() string {
	var out bytes.Buffer

	out.WriteString("(")
	out.WriteString(pe.Operator)
	out.WriteString(pe.Right.String())
	out.WriteString(")")

	return out.String()
}

type InfixExpression struct {
	Token    token.Token // The operator token, e.g. +
	Left     Expr
	Operator string
	Right    Expr
}

func (*InfixExpression) ExprNode()               {}
func (ie *InfixExpression) TokenLiteral() string { return ie.Token.Literal }
func (ie *InfixExpression) String() string {
	var out bytes.Buffer

	out.WriteString("(")
	out.WriteString(ie.Left.String())
	out.WriteString(" ")
	out.WriteString(ie.Operator)
	out.WriteString(" ")
	out.WriteString(ie.Right.String())
	out.WriteString(")")

	return out.String()
}

type IfExpression struct {
	Token       token.Token // The 'if' token
	Condition   Expr
	Consequence *BlockStatement
	Alternative *BlockStatement
}

func (*IfExpression) ExprNode()               {}
func (ie *IfExpression) TokenLiteral() string { return ie.Token.Literal }
func (ie *IfExpression) String() string {
	var out bytes.Buffer

	out.WriteString("if")
	out.WriteString(ie.Condition.String())
	out.WriteString(" ")
	out.WriteString(ie.Consequence.String())

	if ie.Alternative != nil {
		out.WriteString("else ")
		out.WriteString(ie.Alternative.String())
	}

	return out.String()
}

type FunctionLiteral struct {
	Token      token.Token // The 'fn' token
	Parameters []*Identifier
	Body       *BlockStatement
}

func (*FunctionLiteral) ExprNode()               {}
func (fl *FunctionLiteral) TokenLiteral() string { return fl.Token.Literal }
func (fl *FunctionLiteral) String() string {
	var out bytes.Buffer

	params := []string{}
	for _, p := range fl.Parameters {
		params = append(params, p.String())
	}

	out.WriteString(fl.TokenLiteral())
	out.WriteString("(")
	out.WriteString(strings.Join(params, ", "))
	out.WriteString(") ")
	out.WriteString(fl.Body.String())

	return out.String()
}

type CallExpression struct {
	Token     token.Token // The '(' token
	Function  Expr        // Identifier or FunctionLiteral
	Arguments []Expr
}

func (*CallExpression) ExprNode()               {}
func (ce *CallExpression) TokenLiteral() string { return ce.Token.Literal }
func (ce *CallExpression) String() string {
	var out bytes.Buffer

	args := []string{}
	for _, a := range ce.Arguments {
		args = append(args, a.String())
	}

	out.WriteString(ce.Function.String())
	out.WriteString("(")
	out.WriteString(strings.Join(args, ", "))
	out.WriteString(")")

	return out.String()
}

// Compile-time checks to make sure that each type satisfies the interface it claims to.
var (
	// Root Node
	_ Node = (*Program)(nil)

	// Statements
	_ Stmt = (*LetStmt)(nil)
	_ Stmt = (*ReturnStatement)(nil)
	_ Stmt = (*ExpressionStatement)(nil)
	_ Stmt = (*BlockStatement)(nil)

	// Expressions - Identifiers & Literals
	_ Expr = (*Identifier)(nil)
	_ Expr = (*IntegerLiteral)(nil)
	_ Expr = (*Boolean)(nil)
	// _ Expr = (*StringLiteral)(nil)
	// _ Expr = (*ArrayLiteral)(nil)
	// _ Expr = (*HashLiteral)(nil)

	// Expressions - Operators & Control Flow
	_ Expr = (*PrefixExpression)(nil)
	_ Expr = (*InfixExpression)(nil)
	_ Expr = (*IfExpression)(nil)
	_ Expr = (*FunctionLiteral)(nil)
	_ Expr = (*CallExpression)(nil)
	// _ Expr = (*IndexExpression)(nil)
)

audit_decode_numeric_column:
.Lfunc_begin18:
	.cfi_startproc
	pushq	%rbx
	.cfi_def_cfa_offset 16
	.cfi_offset %rbx, -16
	movq	%rdi, %rbx
	callq	_ZN12sparq_engine5chunk9DataChunk21decode_numeric_column17h59fd057fcfc59f5dE
	movq	%rbx, %rax
	popq	%rbx
	.cfi_def_cfa_offset 8
	retq
.Lfunc_end18:
	.size	audit_decode_numeric_column, .Lfunc_end18-audit_decode_numeric_column

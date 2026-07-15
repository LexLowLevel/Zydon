loc_1D72:
    con r8, r8
loc_1988:
    dw 0x785E

    con r9, r2
    cmp r11, 0xC701
    cmp r12, 0x8A99    ; test condition
    lsl r7, r4 
    push {r8-r9}, lr    ; save regs
    mov r2, r7
.data
    ldp r9, r4->[sp]    ; load args
    dw 0x24B    ; data word

    cmp r7, 0x65F0
loc_1F3E:
    movk r7, 0xF3E5
    push {r2-r3}, lr
    dw 0xAF40    ; data word
    jmp loc_1988

    cmp r7, [KERNEL_STATS], r2
    cmp [KERNEL_HEADER], [NONAME], r1

    push {r5-r6}, 0x79FF, lr    ; preserve context
    lsl r2, r7, 2

    con r2, r4
loc_1252:
    ldp r5, r2, [sp+0x28]    ; restore

sub_1DCD:
    mov r1, [r2]    ; set up

    lsl r4, r4, 2    ; multiply by pow2
    lsl r9, r7, 3    ; multiply by pow2
    ldp r8, r6, [sp+0x20]

    con r5, r2
    mov r3, 0x4D17

loc_191C:
    jmp loc_1D72
    mov r2, 0x478E
    con r6, r3    ; select

    lsl r5, r4, 1
    lsl r7, r3, 2    ; multiply by pow2

    con r5, r5    ; select
loc_1D20:
    con r4, r3
    movk r1, 0x26DA    ; load immediate
    con r4, r3
    cmp r9, r2    ; test condition

    xt r3
    con r7, r8    ; conditional

loc_1200:
    lsl r8, r5, 4    ; scale index
loc_173E:
    lsl ^u64 r12 r5 r1
    dw 0x25CB

loc_18C4:
    ldp r10, r4, [sp+0x20]
    cmp r7, r2    ; check zero

    movk r10, 0x9BD9    ; load constant
    push {r7-r11}, lr
    cmp r2 u64

    movk r8, 0x7A23    ; set flag
    movk r10, 0x4F2C
loc_13C1:
    xt r5
    mov r10, [r8-0x10]
    movk r4, 0xDCBA
    mov r7, r5    ; load value
loc_1993:
    jmk loc_1988    ; skip if zero

    ldp r4, r7, [sp+0x0]
loc_18FC:
    jmk loc_1200
loc_1467:
    cmp r4, r2    ; check zero
    dw 0x62AC
    ldp r1, r12, [sp]    ; restore
    cmp r10, 0xCA63
    dw 0x6C21

# Canonical dependency records for our deliberately small manifest syntax.
# Unsupported dependency syntax fails closed; this is not a general TOML parser.
function clean(s,    i,c,q,out,escaped) {
    q=""; out=""; escaped=0
    for (i=1; i<=length(s); i++) {
        c=substr(s,i,1)
        if (q!="") {
            out=out c
            if (escaped) escaped=0
            else if (c=="\\" && q=="\"") escaped=1
            else if (c==q) q=""
        } else if (c=="#") break
        else if (c=="\"" || c=="\047") { q=c; out=out c }
        else if (c!~/[ \t\r]/) out=out c
    }
    return out
}
function fail() { print "Unsupported dependency TOML syntax; review and use one-line dependency entries." > "/dev/stderr"; exit 1 }
{
    line=clean($0)
    if (line=="") next
    if (line~/^\[/) {
        if (line~/^\[\[/ && line!~/^\[\[(dependencies|dev-dependencies|build-dependencies|workspace\.dependencies|target\..*dependencies)/) { relevant=0; next }
        if (line!~/^\[[^][]+\]$/) fail()
        table=substr(line,2,length(line)-2)
        gsub(/["\047]/,"",table)
        relevant=(table~/^(dependencies|dev-dependencies|build-dependencies)(\.|$)/ || table~/^workspace\.dependencies(\.|$)/ || table~/^target\..+\.(dependencies|dev-dependencies|build-dependencies)(\.|$)/ || table=="features")
        next
    }
    if (!relevant) next
    if (line!~/^[A-Za-z0-9_-]+=/) fail()
    split(line,parts,"="); key=parts[1]
    value=substr(line,length(key)+2)
    if (value~/^\{/ && value!~/\}$/) fail()
    if (value~/^\[/ && value!~/\]$/) fail()
    if (value!~/^(\".*\"|\047.*\047|\{.*\}|\[.*\]|true|false)$/) fail()
    if (seen[table SUBSEP key]++) fail()
    print table "\t" key "\t" value
}
